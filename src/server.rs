use crate::event::EventType;
use crate::run::{Run, RunError, RunStatus};
use crate::storage::JsonlStorage;
use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

/// A lightweight, pure standard-library HTTP/1.1 server exposing Kairo's core API.
pub struct HttpServer {
    base_dir: PathBuf,
    addr: String,
    shutdown: Arc<AtomicBool>,
}

impl HttpServer {
    pub fn new(base_dir: impl Into<PathBuf>, addr: impl Into<String>) -> Self {
        Self {
            base_dir: base_dir.into(),
            addr: addr.into(),
            shutdown: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Provides an atomic boolean handle to gracefully stop the server loop.
    pub fn shutdown_handle(&self) -> Arc<AtomicBool> {
        self.shutdown.clone()
    }

    /// Starts the server and listens for incoming HTTP requests.
    pub fn run(&self) -> Result<(), std::io::Error> {
        let listener = TcpListener::bind(&self.addr)?;
        listener.set_nonblocking(true)?;

        println!("Kairo HTTP Server listening on http://{}", self.addr);
        println!("Run data directory: {}", self.base_dir.display());

        while !self.shutdown.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let dir = self.base_dir.clone();
                    thread::spawn(move || {
                        let _ = handle_connection(stream, &dir);
                    });
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => return Err(e),
            }
        }

        Ok(())
    }
}

/// Parses an HTTP request, routes it to the appropriate core operation, and writes the HTTP response.
fn handle_connection(mut stream: TcpStream, base_dir: &Path) -> Result<(), std::io::Error> {
    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(()); // Connection closed by client
    }

    let parts: Vec<&str> = request_line.trim().split_whitespace().collect();
    if parts.len() < 2 {
        return send_response(&mut stream, 400, &json!({"error": "Malformed request line"}));
    }

    let method = parts[0];
    let path = parts[1];

    // Read HTTP headers
    let mut content_length = 0usize;
    loop {
        let mut header_line = String::new();
        if reader.read_line(&mut header_line)? == 0 {
            break;
        }
        let trimmed = header_line.trim();
        if trimmed.is_empty() {
            break; // Blank line marks end of headers
        }
        if let Some((name, val)) = trimmed.split_once(':') {
            if name.trim().eq_ignore_ascii_case("Content-Length") {
                content_length = val.trim().parse().unwrap_or(0);
            }
        }
    }

    // Read request body if present
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body)?;
    }

    // Route request
    route_request(&mut stream, base_dir, method, path, &body)
}

fn route_request(
    stream: &mut TcpStream,
    base_dir: &Path,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<(), std::io::Error> {
    // 1. GET /health
    if method == "GET" && path == "/health" {
        return send_response(
            stream,
            200,
            &json!({
                "status": "ok",
                "service": "kairo",
                "version": "0.1.0"
            }),
        );
    }

    // 2. POST /api/runs (Create a new run)
    if method == "POST" && path == "/api/runs" {
        let payload: serde_json::Value = match serde_json::from_slice(body) {
            Ok(v) => v,
            Err(e) => {
                return send_response(
                    stream,
                    400,
                    &json!({"error": format!("Invalid JSON body: {e}")}),
                )
            }
        };

        let custom_id = payload.get("run_id").and_then(|v| v.as_str());
        let run_id = custom_id
            .map(|s| s.to_string())
            .unwrap_or_else(|| format!("run-{}", uuid::Uuid::new_v4()));

        let run_file = base_dir.join(format!("{run_id}.jsonl"));
        let agent = payload
            .get("agent")
            .and_then(|v| v.as_str())
            .unwrap_or("python-agent");

        let start_payload = json!({
            "agent": agent,
            "initiated_by": "http-api",
            "metadata": payload.get("metadata").cloned().unwrap_or(json!({})),
        });

        match Run::start_with_id(&run_id, &run_file, start_payload) {
            Ok(run) => {
                return send_response(
                    stream,
                    201,
                    &json!({
                        "run_id": run.run_id(),
                        "status": run.status().to_string(),
                        "storage_path": run.storage_path().display().to_string(),
                    }),
                );
            }
            Err(e) => {
                return send_response(
                    stream,
                    500,
                    &json!({"error": format!("Failed to create run: {e}")}),
                );
            }
        }
    }

    // 3. POST /api/runs/:id/events (Record an event)
    if method == "POST" && path.starts_with("/api/runs/") && path.ends_with("/events") {
        let segments: Vec<&str> = path.split('/').collect();
        // Path: ["", "api", "runs", ":id", "events"] -> len = 5
        if segments.len() == 5 {
            let run_id = segments[3];
            let run_path = match Run::find_path(base_dir, run_id) {
                Some(p) => p,
                None => {
                    return send_response(
                        stream,
                        404,
                        &json!({"error": format!("Run '{run_id}' not found")}),
                    )
                }
            };

            // Verify run is currently running
            let run = match Run::load(&run_path) {
                Ok(r) => r,
                Err(e) => {
                    return send_response(
                        stream,
                        500,
                        &json!({"error": format!("Failed to load run: {e}")}),
                    )
                }
            };

            if run.status() != RunStatus::Running {
                return send_response(
                    stream,
                    409,
                    &json!({"error": format!("Cannot record event: run is in '{}' state", run.status())}),
                );
            }

            let req: serde_json::Value = match serde_json::from_slice(body) {
                Ok(v) => v,
                Err(e) => {
                    return send_response(
                        stream,
                        400,
                        &json!({"error": format!("Invalid JSON body: {e}")}),
                    )
                }
            };

            let event_type_str = req.get("event_type").and_then(|v| v.as_str()).unwrap_or("custom");
            let event_payload = req.get("payload").cloned().unwrap_or(json!({}));

            let event_type: EventType = match serde_json::from_value(json!(event_type_str)) {
                Ok(t) => t,
                Err(_) => EventType::Custom(event_type_str.to_string()),
            };

            match run.record(event_type.clone(), event_payload) {
                Ok(event) => {
                    return send_response(
                        stream,
                        200,
                        &json!({
                            "event_id": event.event_id,
                            "run_id": event.run_id,
                            "event_type": event_type_str,
                            "timestamp": event.timestamp,
                            "status": "recorded",
                        }),
                    );
                }
                Err(RunError::InvalidStateTransition { current, attempted }) => {
                    return send_response(
                        stream,
                        409,
                        &json!({"error": format!("Invalid transition: {attempted} while {current}")}),
                    );
                }
                Err(e) => {
                    return send_response(
                        stream,
                        500,
                        &json!({"error": format!("Failed to record event: {e}")}),
                    );
                }
            }
        }
    }

    // 4. POST /api/runs/:id/finish (Complete or fail run)
    if method == "POST" && path.starts_with("/api/runs/") && path.ends_with("/finish") {
        let segments: Vec<&str> = path.split('/').collect();
        if segments.len() == 5 {
            let run_id = segments[3];
            let run_path = match Run::find_path(base_dir, run_id) {
                Some(p) => p,
                None => {
                    return send_response(
                        stream,
                        404,
                        &json!({"error": format!("Run '{run_id}' not found")}),
                    )
                }
            };

            let mut run = match Run::load(&run_path) {
                Ok(r) => r,
                Err(e) => {
                    return send_response(
                        stream,
                        500,
                        &json!({"error": format!("Failed to load run: {e}")}),
                    )
                }
            };

            let req: serde_json::Value = match serde_json::from_slice(body) {
                Ok(v) => v,
                Err(e) => {
                    return send_response(
                        stream,
                        400,
                        &json!({"error": format!("Invalid JSON body: {e}")}),
                    )
                }
            };

            let status_req = req.get("status").and_then(|v| v.as_str()).unwrap_or("completed");
            let summary = req.get("summary").cloned().unwrap_or(json!({}));

            let result = if status_req == "failed" {
                run.fail(summary)
            } else {
                run.complete(summary)
            };

            match result {
                Ok(ev) => {
                    return send_response(
                        stream,
                        200,
                        &json!({
                            "run_id": run_id,
                            "status": run.status().to_string(),
                            "event_id": ev.event_id,
                        }),
                    );
                }
                Err(RunError::InvalidStateTransition { current, attempted }) => {
                    return send_response(
                        stream,
                        409,
                        &json!({"error": format!("Cannot finish: run is already in '{current}' state (attempted '{attempted}')")}),
                    );
                }
                Err(e) => {
                    return send_response(
                        stream,
                        500,
                        &json!({"error": format!("Failed to finish run: {e}")}),
                    );
                }
            }
        }
    }

    // 5. GET /api/runs/:id (Get run details and events)
    if method == "GET" && path.starts_with("/api/runs/") {
        let segments: Vec<&str> = path.split('/').collect();
        if segments.len() == 4 {
            let run_id = segments[3];
            let run_path = match Run::find_path(base_dir, run_id) {
                Some(p) => p,
                None => {
                    return send_response(
                        stream,
                        404,
                        &json!({"error": format!("Run '{run_id}' not found")}),
                    )
                }
            };

            let run = match Run::load(&run_path) {
                Ok(r) => r,
                Err(e) => {
                    return send_response(
                        stream,
                        500,
                        &json!({"error": format!("Failed to load run: {e}")}),
                    )
                }
            };

            let events = match JsonlStorage::read_events(&run_path) {
                Ok(evs) => evs,
                Err(e) => {
                    return send_response(
                        stream,
                        500,
                        &json!({"error": format!("Failed to read events: {e}")}),
                    )
                }
            };

            return send_response(
                stream,
                200,
                &json!({
                    "run_id": run.run_id(),
                    "status": run.status().to_string(),
                    "event_count": events.len(),
                    "events": events,
                }),
            );
        }
    }

    // Fallthrough: 404 Not Found
    send_response(stream, 404, &json!({"error": format!("Endpoint not found: {method} {path}")}))
}

/// Helper function to serialize JSON response and write standard HTTP headers.
fn send_response(
    stream: &mut TcpStream,
    status_code: u16,
    body: &serde_json::Value,
) -> Result<(), std::io::Error> {
    let status_text = match status_code {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        500 => "Internal Server Error",
        _ => "Unknown",
    };

    let body_bytes = serde_json::to_vec_pretty(body).unwrap_or_default();
    let header = format!(
        "HTTP/1.1 {status_code} {status_text}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body_bytes.len()
    );

    stream.write_all(header.as_bytes())?;
    stream.write_all(&body_bytes)?;
    stream.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::net::TcpStream;
    use std::time::Duration;

    fn get_free_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    #[test]
    fn test_http_api_endpoints() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("kairo_test_server_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();

        let port = get_free_port();
        let addr = format!("127.0.0.1:{port}");
        let server = HttpServer::new(dir.clone(), addr.clone());
        let shutdown = server.shutdown_handle();

        // Spawn server thread
        let srv_handle = thread::spawn(move || {
            server.run().unwrap();
        });

        thread::sleep(Duration::from_millis(50));

        // 1. Health check
        let mut client = TcpStream::connect(&addr).unwrap();
        client.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        let mut resp = String::new();
        client.read_to_string(&mut resp).unwrap();
        assert!(resp.contains("200 OK"));
        assert!(resp.contains("\"status\": \"ok\""));

        // 2. Create Run
        let mut client = TcpStream::connect(&addr).unwrap();
        let req_body = r#"{"run_id": "run-py-test", "agent": "test-bot"}"#;
        let req = format!(
            "POST /api/runs HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
            req_body.len(),
            req_body
        );
        client.write_all(req.as_bytes()).unwrap();
        let mut resp = String::new();
        client.read_to_string(&mut resp).unwrap();
        assert!(resp.contains("201 Created"));
        assert!(resp.contains("run-py-test"));

        // 3. Record Event
        let mut client = TcpStream::connect(&addr).unwrap();
        let ev_body = r#"{"event_type": "tool.completed", "payload": {"result": 42}}"#;
        let req = format!(
            "POST /api/runs/run-py-test/events HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
            ev_body.len(),
            ev_body
        );
        client.write_all(req.as_bytes()).unwrap();
        let mut resp = String::new();
        client.read_to_string(&mut resp).unwrap();
        assert!(resp.contains("200 OK"));
        assert!(resp.contains("\"status\": \"recorded\""));

        // 4. Finish Run
        let mut client = TcpStream::connect(&addr).unwrap();
        let fin_body = r#"{"status": "completed", "summary": {"steps": 1}}"#;
        let req = format!(
            "POST /api/runs/run-py-test/finish HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
            fin_body.len(),
            fin_body
        );
        client.write_all(req.as_bytes()).unwrap();
        let mut resp = String::new();
        client.read_to_string(&mut resp).unwrap();
        assert!(resp.contains("200 OK"));
        assert!(resp.contains("\"completed\""));

        // 5. Duplicate finish returns 409 Conflict
        let mut client = TcpStream::connect(&addr).unwrap();
        let req = format!(
            "POST /api/runs/run-py-test/finish HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{}",
            fin_body.len(),
            fin_body
        );
        client.write_all(req.as_bytes()).unwrap();
        let mut resp = String::new();
        client.read_to_string(&mut resp).unwrap();
        assert!(resp.contains("409 Conflict"));

        // Shutdown server
        shutdown.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(&addr); // Wake accept
        srv_handle.join().unwrap();

        let _ = fs::remove_dir_all(dir);
    }
}
