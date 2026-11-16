use crate::event::{Event, EventType};
use crate::eval::case::EvalCase;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// Provider-supplied token consumption measurements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
}

impl TokenUsage {
    /// Extracts token usage from execution events if and only if the provider supplied it.
    pub fn extract_from_events(events: &[Event]) -> Option<Self> {
        let mut prompt_acc = 0u64;
        let mut completion_acc = 0u64;
        let mut total_acc = 0u64;
        let mut found_any = false;

        for ev in events {
            // Check top-level or nested usage objects
            let usage_val = ev
                .payload
                .get("usage")
                .or_else(|| ev.payload.get("token_usage"))
                .or_else(|| ev.payload.get("tokens"));

            if let Some(usage) = usage_val {
                if let Some(p) = usage.get("prompt_tokens").and_then(|v| v.as_u64()) {
                    prompt_acc += p;
                    found_any = true;
                }
                if let Some(c) = usage.get("completion_tokens").and_then(|v| v.as_u64()) {
                    completion_acc += c;
                    found_any = true;
                }
                if let Some(t) = usage.get("total_tokens").and_then(|v| v.as_u64()) {
                    total_acc += t;
                    found_any = true;
                }
            } else {
                // Check direct fields in summary or payload
                if let Some(t) = ev.payload.get("total_tokens").and_then(|v| v.as_u64()) {
                    total_acc += t;
                    found_any = true;
                }
            }
        }

        if !found_any {
            return None;
        }

        let total = if total_acc > 0 {
            total_acc
        } else {
            prompt_acc + completion_acc
        };

        Some(Self {
            prompt_tokens: if prompt_acc > 0 { Some(prompt_acc) } else { None },
            completion_tokens: if completion_acc > 0 {
                Some(completion_acc)
            } else {
                None
            },
            total_tokens: if total > 0 { Some(total) } else { None },
        })
    }
}

/// The outcome of evaluating an individual run against an evaluation case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalResult {
    pub case_id: String,
    pub run_id: String,
    pub passed: bool,
    pub actual_status: String,
    pub actual_output: Value,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_usage: Option<TokenUsage>,
    pub failures: Vec<String>,
}

/// Evaluates a recorded run's event sequence against an evaluation case.
pub fn evaluate_run(case: &EvalCase, run_id: &str, events: &[Event]) -> EvalResult {
    let mut failures = Vec::new();

    // 1. Determine actual status
    let mut actual_status = "running".to_string();
    let mut actual_output = Value::Null;

    for ev in events {
        match ev.event_type {
            EventType::RunCompleted => {
                actual_status = "completed".to_string();
                if let Some(res) = ev.payload.get("result") {
                    actual_output = res.clone();
                } else if let Some(out) = ev.payload.get("output") {
                    actual_output = out.clone();
                }
            }
            EventType::RunFailed => {
                actual_status = "failed".to_string();
                if let Some(err) = ev.payload.get("error") {
                    actual_output = err.clone();
                }
            }
            _ => {}
        }
    }

    // Fall back to final LLM response if output was not in run.completed
    if actual_output.is_null() {
        for ev in events.iter().rev() {
            if ev.event_type == EventType::LlmResponse {
                if let Some(resp) = ev.payload.get("response") {
                    actual_output = resp.clone();
                    break;
                }
            }
        }
    }

    // 2. Compare run status
    if actual_status != case.expected.status {
        failures.push(format!(
            "Status mismatch: expected '{}', got '{}'",
            case.expected.status, actual_status
        ));
    }

    // 3. Compare observable output
    if !case.expected.output_matcher.matches(&actual_output) {
        failures.push(format!(
            "Output mismatch: expected {}, got '{actual_output}'",
            case.expected.output_matcher.display_expected()
        ));
    }

    // 4. Verify required tool calls
    let mut tools_used = Vec::new();
    for ev in events {
        match ev.event_type {
            EventType::ToolStarted | EventType::ToolCompleted => {
                if let Some(t) = ev.payload.get("tool").and_then(|v| v.as_str()) {
                    let tool_str = t.to_string();
                    if !tools_used.contains(&tool_str) {
                        tools_used.push(tool_str);
                    }
                }
            }
            EventType::Custom(ref name) if name == "tool.call" => {
                if let Some(t) = ev.payload.get("tool").and_then(|v| v.as_str()) {
                    let tool_str = t.to_string();
                    if !tools_used.contains(&tool_str) {
                        tools_used.push(tool_str);
                    }
                }
            }
            _ => {}
        }
    }

    for req_tool in &case.expected.required_tools {
        if !tools_used.contains(req_tool) {
            failures.push(format!("Missing required tool call: '{req_tool}'"));
        }
    }

    // 5. Measure duration
    let duration_ms = calculate_events_duration_ms(events);
    if let Some(max_dur) = case.expected.max_duration_ms {
        if duration_ms > max_dur {
            failures.push(format!(
                "Execution duration exceeded limit: {duration_ms}ms > {max_dur}ms"
            ));
        }
    }

    // 6. Record token usage (only if provider supplied it)
    let token_usage = TokenUsage::extract_from_events(events);
    if let (Some(max_tok), Some(usage)) = (case.expected.max_tokens, &token_usage) {
        if let Some(total) = usage.total_tokens {
            if total > max_tok {
                failures.push(format!("Token usage exceeded limit: {total} > {max_tok}"));
            }
        }
    }

    let passed = failures.is_empty();

    EvalResult {
        case_id: case.id.clone(),
        run_id: run_id.to_string(),
        passed,
        actual_status,
        actual_output,
        duration_ms,
        token_usage,
        failures,
    }
}

/// A concise evaluation report summarizing benchmark results.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalReport {
    pub report_id: String,
    pub timestamp: String,
    pub total_cases: usize,
    pub passed_cases: usize,
    pub failed_cases: usize,
    pub pass_rate: f64,
    pub total_duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
    pub results: Vec<EvalResult>,
}

impl EvalReport {
    pub fn new(report_id: impl Into<String>, results: Vec<EvalResult>) -> Self {
        let report_id = report_id.into();
        let total_cases = results.len();
        let passed_cases = results.iter().filter(|r| r.passed).count();
        let failed_cases = total_cases - passed_cases;
        let pass_rate = if total_cases > 0 {
            (passed_cases as f64 / total_cases as f64) * 100.0
        } else {
            0.0
        };

        let total_duration_ms = results.iter().map(|r| r.duration_ms).sum();

        let mut token_sum = 0u64;
        let mut has_tokens = false;
        for r in &results {
            if let Some(ref tu) = r.token_usage {
                if let Some(tot) = tu.total_tokens {
                    token_sum += tot;
                    has_tokens = true;
                }
            }
        }

        Self {
            report_id,
            timestamp: crate::event::current_utc_iso8601(),
            total_cases,
            passed_cases,
            failed_cases,
            pass_rate,
            total_duration_ms,
            total_tokens: if has_tokens { Some(token_sum) } else { None },
            results,
        }
    }

    /// Saves the report as JSON in `.kairo/evals/reports/`.
    pub fn save_to_dir(&self, base_dir: impl AsRef<Path>) -> Result<PathBuf, String> {
        let reports_dir = base_dir.as_ref().join("evals").join("reports");
        fs::create_dir_all(&reports_dir)
            .map_err(|e| format!("Failed to create reports directory: {e}"))?;

        let file_path = reports_dir.join(format!("{}.json", self.report_id));
        let json_data = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize report: {e}"))?;

        fs::write(&file_path, json_data)
            .map_err(|e| format!("Failed to write report file: {e}"))?;

        Ok(file_path)
    }

    /// Loads a report from file.
    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self, String> {
        let data = fs::read_to_string(path.as_ref())
            .map_err(|e| format!("Failed to read report file: {e}"))?;
        serde_json::from_str(&data).map_err(|e| format!("Failed to parse report JSON: {e}"))
    }
}

/// Helper calculating execution duration from first and last event timestamps.
fn calculate_events_duration_ms(events: &[Event]) -> u64 {
    if events.len() < 2 {
        return 0;
    }

    let first = &events[0];
    let last = &events[events.len() - 1];

    let t1 = parse_iso_timestamp_secs(&first.timestamp);
    let t2 = parse_iso_timestamp_secs(&last.timestamp);

    if let (Some(s1), Some(s2)) = (t1, t2) {
        if s2 >= s1 {
            return (s2 - s1) * 1000;
        }
    }

    0
}

/// Simple standard-library ISO-8601 / RFC-3339 timestamp parser to epoch seconds.
fn parse_iso_timestamp_secs(iso: &str) -> Option<u64> {
    // Format: "YYYY-MM-DDTHH:MM:SSZ" (length 20)
    let s = iso.trim();
    if s.len() < 19 {
        return None;
    }

    let year: u64 = s[0..4].parse().ok()?;
    let month: u64 = s[5..7].parse().ok()?;
    let day: u64 = s[8..10].parse().ok()?;
    let hour: u64 = s[11..13].parse().ok()?;
    let min: u64 = s[14..16].parse().ok()?;
    let sec: u64 = s[17..19].parse().ok()?;

    // Days since epoch approximation for duration diff
    let days = (year.saturating_sub(1970)) * 365 + (month.saturating_sub(1)) * 30 + day;
    Some(days * 86400 + hour * 3600 + min * 60 + sec)
}
