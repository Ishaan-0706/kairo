use crate::event::{Event, EventType};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

/// Criteria for matching actual output against expected output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum OutputMatcher {
    /// Exact match on the JSON value or primitive.
    Exact(Value),
    /// String contains substring (works on string values or stringified JSON).
    Contains(String),
    /// Any output is accepted (used when only tool usage or status is being verified).
    Any,
}

impl OutputMatcher {
    pub fn matches(&self, actual: &Value) -> bool {
        match self {
            OutputMatcher::Exact(expected) => actual == expected,
            OutputMatcher::Contains(substr) => match actual {
                Value::String(s) => s.contains(substr),
                other => other.to_string().contains(substr),
            },
            OutputMatcher::Any => true,
        }
    }

    pub fn display_expected(&self) -> String {
        match self {
            OutputMatcher::Exact(v) => format!("exact: {v}"),
            OutputMatcher::Contains(s) => format!("contains: \"{s}\""),
            OutputMatcher::Any => "any".to_string(),
        }
    }
}

/// The expected outcome definition for an evaluation case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpectedOutcome {
    /// Expected final run status (typically "completed")
    pub status: String,
    /// Matching rule for the final output
    pub output_matcher: OutputMatcher,
    /// List of tool names that must be called during execution
    pub required_tools: Vec<String>,
    /// Maximum allowed execution duration in milliseconds
    pub max_duration_ms: Option<u64>,
    /// Maximum allowed total token usage (if provider supplied)
    pub max_tokens: Option<u64>,
}

impl Default for ExpectedOutcome {
    fn default() -> Self {
        Self {
            status: "completed".to_string(),
            output_matcher: OutputMatcher::Any,
            required_tools: Vec::new(),
            max_duration_ms: None,
            max_tokens: None,
        }
    }
}

/// A repeatable evaluation test case.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvalCase {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Run ID from which this evaluation example was derived (if recorded)
    pub source_run_id: Option<String>,
    /// Input prompt or parameters supplied to the agent
    pub input: Value,
    /// Expected observable outcomes
    pub expected: ExpectedOutcome,
}

impl EvalCase {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        input: Value,
        expected: ExpectedOutcome,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            source_run_id: None,
            input,
            expected,
        }
    }

    /// Derives an evaluation case automatically from an existing recorded run.
    pub fn from_run(run_id: &str, events: &[Event]) -> Result<Self, String> {
        if events.is_empty() {
            return Err(format!("Run '{run_id}' has no recorded events"));
        }

        // 1. Extract input prompt/goal
        let mut input = Value::Null;
        for ev in events {
            if ev.event_type == EventType::RunStarted {
                if let Some(meta) = ev.payload.get("metadata") {
                    if let Some(goal) = meta.get("goal") {
                        input = goal.clone();
                        break;
                    }
                }
            } else if ev.event_type == EventType::LlmRequest {
                if let Some(prompt) = ev.payload.get("prompt") {
                    input = prompt.clone();
                    break;
                }
            }
        }

        // 2. Collect unique tools invoked
        let mut required_tools = Vec::new();
        for ev in events {
            match ev.event_type {
                EventType::ToolStarted | EventType::ToolCompleted => {
                    if let Some(tool) = ev.payload.get("tool").and_then(|v| v.as_str()) {
                        let tool_str = tool.to_string();
                        if !required_tools.contains(&tool_str) {
                            required_tools.push(tool_str);
                        }
                    }
                }
                EventType::Custom(ref name) if name == "tool.call" => {
                    if let Some(tool) = ev.payload.get("tool").and_then(|v| v.as_str()) {
                        let tool_str = tool.to_string();
                        if !required_tools.contains(&tool_str) {
                            required_tools.push(tool_str);
                        }
                    }
                }
                _ => {}
            }
        }

        // 3. Determine expected status and output
        let mut status = "completed".to_string();
        let mut final_output = Value::Null;

        for ev in events.iter().rev() {
            match ev.event_type {
                EventType::RunCompleted => {
                    status = "completed".to_string();
                    if let Some(res) = ev.payload.get("result") {
                        final_output = res.clone();
                        break;
                    }
                }
                EventType::RunFailed => {
                    status = "failed".to_string();
                    if let Some(err) = ev.payload.get("error") {
                        final_output = err.clone();
                        break;
                    }
                }
                EventType::LlmResponse if final_output.is_null() => {
                    if let Some(resp) = ev.payload.get("response") {
                        final_output = resp.clone();
                    }
                }
                _ => {}
            }
        }

        let output_matcher = if !final_output.is_null() {
            OutputMatcher::Exact(final_output)
        } else {
            OutputMatcher::Any
        };

        Ok(Self {
            id: format!("eval-{run_id}"),
            name: format!("Evaluation for {run_id}"),
            description: format!("Auto-generated evaluation case from recorded run '{run_id}'"),
            source_run_id: Some(run_id.to_string()),
            input,
            expected: ExpectedOutcome {
                status,
                output_matcher,
                required_tools,
                max_duration_ms: None,
                max_tokens: None,
            },
        })
    }

    /// Saves the evaluation case to a directory as a JSON file.
    pub fn save_to_dir(&self, base_dir: impl AsRef<Path>) -> Result<PathBuf, String> {
        let cases_dir = base_dir.as_ref().join("evals").join("cases");
        fs::create_dir_all(&cases_dir)
            .map_err(|e| format!("Failed to create evals directory: {e}"))?;

        let file_path = cases_dir.join(format!("{}.json", self.id));
        let json_data = serde_json::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize eval case: {e}"))?;

        fs::write(&file_path, json_data)
            .map_err(|e| format!("Failed to write eval case file: {e}"))?;

        Ok(file_path)
    }

    /// Loads an evaluation case from a JSON file.
    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self, String> {
        let data = fs::read_to_string(path.as_ref())
            .map_err(|e| format!("Failed to read eval case file: {e}"))?;
        serde_json::from_str(&data).map_err(|e| format!("Failed to parse eval case JSON: {e}"))
    }

    /// Lists all evaluation cases in the evals directory.
    pub fn list_cases(base_dir: impl AsRef<Path>) -> Result<Vec<Self>, String> {
        let cases_dir = base_dir.as_ref().join("evals").join("cases");
        if !cases_dir.exists() {
            return Ok(Vec::new());
        }

        let mut cases = Vec::new();
        let entries = fs::read_dir(cases_dir)
            .map_err(|e| format!("Failed to read evals directory: {e}"))?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                if let Ok(case) = Self::load_from_file(&path) {
                    cases.push(case);
                }
            }
        }

        cases.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(cases)
    }
}
