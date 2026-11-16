use crate::event::EventType;
use crate::run::{Run, RunError, RunStatus};
use crate::storage::JsonlStorage;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde_json::{json, Value};
use std::path::PathBuf;

/// Shared application state passed to all Axum request handlers.
#[derive(Clone, Debug)]
pub struct AppState {
    pub base_dir: PathBuf,
}

/// Builds the Axum router with all API routes.
pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/api/runs", post(create_run_handler))
        .route("/api/runs/{id}/events", post(record_event_handler))
        .route("/api/runs/{id}/finish", post(finish_run_handler))
        .route("/api/runs/{id}", get(get_run_handler))
        .with_state(state)
}

/// GET /health - Service health check
async fn health_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({
            "status": "ok",
            "service": "kairo",
            "version": "0.1.0"
        })),
    )
}

/// POST /api/runs - Create a new run
async fn create_run_handler(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let custom_id = body.get("run_id").and_then(|v| v.as_str());
    let run_id = custom_id
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("run-{}", uuid::Uuid::new_v4()));

    let run_file = state.base_dir.join(format!("{run_id}.jsonl"));
    let agent = body
        .get("agent")
        .and_then(|v| v.as_str())
        .unwrap_or("python-agent");

    let start_payload = json!({
        "agent": agent,
        "initiated_by": "http-api",
        "metadata": body.get("metadata").cloned().unwrap_or(json!({})),
    });

    match Run::start_with_id(&run_id, &run_file, start_payload) {
        Ok(run) => (
            StatusCode::CREATED,
            Json(json!({
                "run_id": run.run_id(),
                "status": run.status().to_string(),
                "storage_path": run.storage_path().display().to_string(),
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to create run: {e}")})),
        ),
    }
}

/// POST /api/runs/{id}/events - Record an event to an active run
async fn record_event_handler(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let run_path = match Run::find_path(&state.base_dir, &run_id) {
        Some(p) => p,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Run '{run_id}' not found")})),
            );
        }
    };

    let run = match Run::load(&run_path) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to load run: {e}")})),
            );
        }
    };

    if run.status() != RunStatus::Running {
        return (
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!("Cannot record event: run is in '{}' state", run.status())
            })),
        );
    }

    let event_type_str = body
        .get("event_type")
        .and_then(|v| v.as_str())
        .unwrap_or("custom");
    let event_payload = body.get("payload").cloned().unwrap_or(json!({}));

    let event_type: EventType = match serde_json::from_value(json!(event_type_str)) {
        Ok(t) => t,
        Err(_) => EventType::Custom(event_type_str.to_string()),
    };

    match run.record(event_type, event_payload) {
        Ok(event) => (
            StatusCode::OK,
            Json(json!({
                "event_id": event.event_id,
                "run_id": event.run_id,
                "event_type": event_type_str,
                "timestamp": event.timestamp,
                "status": "recorded",
            })),
        ),
        Err(RunError::InvalidStateTransition { current, attempted }) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!("Invalid transition: {attempted} while {current}")
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to record event: {e}")})),
        ),
    }
}

/// POST /api/runs/{id}/finish - Finalize run as completed or failed
async fn finish_run_handler(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let run_path = match Run::find_path(&state.base_dir, &run_id) {
        Some(p) => p,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Run '{run_id}' not found")})),
            );
        }
    };

    let mut run = match Run::load(&run_path) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to load run: {e}")})),
            );
        }
    };

    let status_req = body
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("completed");
    let summary = body.get("summary").cloned().unwrap_or(json!({}));

    let result = if status_req == "failed" {
        run.fail(summary)
    } else {
        run.complete(summary)
    };

    match result {
        Ok(ev) => (
            StatusCode::OK,
            Json(json!({
                "run_id": run_id,
                "status": run.status().to_string(),
                "event_id": ev.event_id,
            })),
        ),
        Err(RunError::InvalidStateTransition { current, attempted }) => (
            StatusCode::CONFLICT,
            Json(json!({
                "error": format!(
                    "Cannot finish: run is already in '{current}' state (attempted '{attempted}')"
                )
            })),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to finish run: {e}")})),
        ),
    }
}

/// GET /api/runs/{id} - Inspect run status and events
async fn get_run_handler(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    let run_path = match Run::find_path(&state.base_dir, &run_id) {
        Some(p) => p,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Run '{run_id}' not found")})),
            );
        }
    };

    let run = match Run::load(&run_path) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to load run: {e}")})),
            );
        }
    };

    let events = match JsonlStorage::read_events(&run_path) {
        Ok(evs) => evs,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to read events: {e}")})),
            );
        }
    };

    (
        StatusCode::OK,
        Json(json!({
            "run_id": run.run_id(),
            "status": run.status().to_string(),
            "event_count": events.len(),
            "events": events,
        })),
    )
}

/// High-performance Axum HTTP server hosting Kairo's core API.
pub struct HttpServer {
    base_dir: PathBuf,
    addr: String,
}

impl HttpServer {
    pub fn new(base_dir: impl Into<PathBuf>, addr: impl Into<String>) -> Self {
        Self {
            base_dir: base_dir.into(),
            addr: addr.into(),
        }
    }

    /// Synchronous blocking run entrypoint, constructing a Tokio runtime internally
    /// to support CLI invocation (`kairo serve`).
    pub fn run(&self) -> Result<(), std::io::Error> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;

        rt.block_on(self.run_async())
    }

    /// Asynchronous execution entrypoint.
    pub async fn run_async(&self) -> Result<(), std::io::Error> {
        let state = AppState {
            base_dir: self.base_dir.clone(),
        };
        let app = create_router(state);

        let listener = tokio::net::TcpListener::bind(&self.addr).await?;
        println!("Kairo HTTP Server (Axum) listening on http://{}", self.addr);
        println!("Run data directory: {}", self.base_dir.display());

        axum::serve(listener, app).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::net::TcpListener;
    use std::time::Duration;

    fn get_free_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    }

    #[tokio::test]
    async fn test_http_api_endpoints() {
        let mut dir = std::env::temp_dir();
        dir.push(format!("kairo_axum_test_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();

        let port = get_free_port();
        let addr = format!("127.0.0.1:{port}");
        let state = AppState {
            base_dir: dir.clone(),
        };
        let app = create_router(state);

        let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();

        // Spawn Axum server on Tokio background task
        let srv_handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let client = reqwest_or_tcp(&addr);

        // 1. GET /health
        let (status, body) = client.get("/health").await;
        assert_eq!(status, 200);
        assert_eq!(body["status"], "ok");
        assert_eq!(body["service"], "kairo");

        // 2. POST /api/runs (Create Run)
        let (status, body) = client
            .post(
                "/api/runs",
                &json!({"run_id": "run-axum-test", "agent": "axum-bot"}),
            )
            .await;
        assert_eq!(status, 201);
        assert_eq!(body["run_id"], "run-axum-test");
        assert_eq!(body["status"], "running");

        // 3. POST /api/runs/:id/events (Record Event)
        let (status, body) = client
            .post(
                "/api/runs/run-axum-test/events",
                &json!({
                    "event_type": "tool.completed",
                    "payload": {"result": 42}
                }),
            )
            .await;
        assert_eq!(status, 200);
        assert_eq!(body["status"], "recorded");
        assert_eq!(body["event_type"], "tool.completed");

        // 4. POST /api/runs/:id/finish (Complete Run)
        let (status, body) = client
            .post(
                "/api/runs/run-axum-test/finish",
                &json!({"status": "completed", "summary": {"steps": 1}}),
            )
            .await;
        assert_eq!(status, 200);
        assert_eq!(body["status"], "completed");

        // 5. POST /api/runs/:id/finish again (Returns 409 Conflict)
        let (status, body) = client
            .post(
                "/api/runs/run-axum-test/finish",
                &json!({"status": "completed"}),
            )
            .await;
        assert_eq!(status, 409);
        assert!(body["error"].as_str().unwrap().contains("Cannot finish"));

        // 6. GET /api/runs/:id (Inspect Run)
        let (status, body) = client.get("/api/runs/run-axum-test").await;
        assert_eq!(status, 200);
        assert_eq!(body["event_count"], 3);

        srv_handle.abort();
        let _ = fs::remove_dir_all(dir);
    }

    struct SimpleClient {
        addr: String,
    }

    fn reqwest_or_tcp(addr: &str) -> SimpleClient {
        SimpleClient {
            addr: addr.to_string(),
        }
    }

    impl SimpleClient {
        async fn get(&self, path: &str) -> (u16, Value) {
            self.request("GET", path, None).await
        }

        async fn post(&self, path: &str, body: &Value) -> (u16, Value) {
            self.request("POST", path, Some(body)).await
        }

        async fn request(&self, method: &str, path: &str, body: Option<&Value>) -> (u16, Value) {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut stream = tokio::net::TcpStream::connect(&self.addr).await.unwrap();

            let (body_str, content_length) = match body {
                Some(b) => {
                    let s = b.to_string();
                    let len = s.len();
                    (s, len)
                }
                None => (String::new(), 0),
            };

            let req = format!(
                "{method} {path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {content_length}\r\nConnection: close\r\n\r\n{body_str}",
                self.addr
            );

            stream.write_all(req.as_bytes()).await.unwrap();

            let mut resp = String::new();
            stream.read_to_string(&mut resp).await.unwrap();

            let (header_part, body_part) = resp.split_once("\r\n\r\n").unwrap_or(("", ""));
            let status_code: u16 = header_part
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(500);

            let json_val = serde_json::from_str(body_part).unwrap_or(json!({}));
            (status_code, json_val)
        }
    }
}
