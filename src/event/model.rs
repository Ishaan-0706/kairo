use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use uuid::Uuid;

/// Represents the classification of an observable agent lifecycle or execution event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventType {
    #[serde(rename = "run.started")]
    RunStarted,
    #[serde(rename = "run.completed")]
    RunCompleted,
    #[serde(rename = "run.failed")]
    RunFailed,
    #[serde(rename = "llm.request")]
    LlmRequest,
    #[serde(rename = "llm.response")]
    LlmResponse,
    #[serde(rename = "tool.started")]
    ToolStarted,
    #[serde(rename = "tool.completed")]
    ToolCompleted,
    #[serde(rename = "tool.failed")]
    ToolFailed,
    /// Extensible catch-all for application-specific or future event types.
    #[serde(untagged)]
    Custom(String),
}

/// The atomic record of an execution event within a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub event_id: String,
    pub run_id: String,
    pub timestamp: String,
    pub event_type: EventType,
    pub payload: serde_json::Value,
}

impl Event {
    /// Creates a new Event with an auto-generated UUIDv4 and UTC ISO-8601 timestamp.
    pub fn new(run_id: impl Into<String>, event_type: EventType, payload: serde_json::Value) -> Self {
        Self {
            event_id: Uuid::new_v4().to_string(),
            run_id: run_id.into(),
            timestamp: current_utc_iso8601(),
            event_type,
            payload,
        }
    }
}

/// Helper returning the current UTC time as an ISO-8601 RFC-3339 string.
pub fn current_utc_iso8601() -> String {
    format_system_time(SystemTime::now())
}

/// Pure standard-library epoch to UTC calendar formatter.
fn format_system_time(time: SystemTime) -> String {
    let dur = time.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = dur.as_secs();
    let z = (secs / 86400) as i64 + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let time_of_day = secs % 86400;
    let hour = time_of_day / 3600;
    let min = (time_of_day % 3600) / 60;
    let sec = time_of_day % 60;
    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{min:02}:{sec:02}Z")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_event_round_trip_serialization() {
        let payload = json!({
            "tool": "read_file",
            "path": "src/main.rs",
            "status": "success"
        });
        let original = Event::new("run-123", EventType::ToolCompleted, payload);

        // Serialize to JSON string
        let json_str = serde_json::to_string_pretty(&original).expect("Serialization failed");

        // Deserialize back to Event
        let deserialized: Event = serde_json::from_str(&json_str).expect("Deserialization failed");

        assert_eq!(original, deserialized);
        assert_eq!(deserialized.run_id, "run-123");
        assert_eq!(deserialized.event_type, EventType::ToolCompleted);
    }

    #[test]
    fn test_event_id_is_valid_uuid() {
        let event = Event::new("run-abc", EventType::RunStarted, json!({}));
        assert!(Uuid::parse_str(&event.event_id).is_ok());
    }

    #[test]
    fn test_custom_event_type() {
        let event = Event::new(
            "run-xyz",
            EventType::Custom("custom.agent.step".to_string()),
            json!({"step": 1}),
        );
        let json_val = serde_json::to_value(&event).expect("to_value failed");
        assert_eq!(json_val["event_type"], "custom.agent.step");

        let deserialized: Event = serde_json::from_value(json_val).expect("deserialization failed");
        assert_eq!(
            deserialized.event_type,
            EventType::Custom("custom.agent.step".to_string())
        );
    }

    #[test]
    fn test_deserialize_incomplete_json_fails() {
        // Missing "run_id" and "payload"
        let broken_json = r#"{"event_id": "test", "event_type": "run.started"}"#;
        let result: Result<Event, _> = serde_json::from_str(broken_json);
        assert!(result.is_err());
    }
}

