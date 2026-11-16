use crate::event::{Event, EventType};
use crate::run::Run;
use crate::storage::{JsonlStorage, StorageError};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::Path;

/// Specific reasons why a replayed execution diverged from the recorded log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Divergence {
    /// Expected a specific event type, but the consumer emitted a different one.
    TypeMismatch {
        index: usize,
        expected: EventType,
        actual: EventType,
    },
    /// The event payload differed from what was recorded.
    PayloadMismatch {
        index: usize,
        expected: serde_json::Value,
        actual: serde_json::Value,
    },
    /// An action was requested, but the expected recorded response event was missing.
    MissingResponse {
        index: usize,
        expected_type: EventType,
    },
    /// Consumer requested an event/response, but the recorded log has ended.
    LogExhausted {
        index: usize,
        attempted: String,
    },
    /// Consumer finished early while recorded events remain unconsumed.
    UnconsumedEvents {
        remaining_count: usize,
        next_event_type: EventType,
    },
}

impl std::fmt::Display for Divergence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Divergence::TypeMismatch { index, expected, actual } => {
                write!(f, "Event #{index} type mismatch: expected {expected:?}, got {actual:?}")
            }
            Divergence::PayloadMismatch { index, expected, actual } => {
                write!(f, "Event #{index} payload mismatch: expected {expected}, got {actual}")
            }
            Divergence::MissingResponse { index, expected_type } => {
                write!(f, "Event #{index} missing response: expected {expected_type:?}")
            }
            Divergence::LogExhausted { index, attempted } => {
                write!(f, "Log exhausted at event #{index}: cannot perform '{attempted}'")
            }
            Divergence::UnconsumedEvents { remaining_count, next_event_type } => {
                write!(
                    f,
                    "Replay ended prematurely with {remaining_count} unconsumed events (next: {next_event_type:?})"
                )
            }
        }
    }
}

impl std::error::Error for Divergence {}

/// Summary report produced at the conclusion of a replay session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayReport {
    pub run_id: String,
    pub total_recorded_events: usize,
    pub matched_events: usize,
    pub divergences: Vec<Divergence>,
}

impl ReplayReport {
    /// Returns true if all events were replayed in sequence with zero divergences.
    pub fn is_success(&self) -> bool {
        self.divergences.is_empty()
    }
}

/// The Replay Engine reads recorded execution events and provides saved
/// responses to consumers without contacting external services or running commands.
pub struct ReplayEngine {
    run_id: String,
    events: Vec<Event>,
    cursor: usize,
    matched_events: usize,
    divergences: Vec<Divergence>,
}

impl ReplayEngine {
    /// Loads a recorded run directly from a JSONL file.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let events = JsonlStorage::read_events(path)?;
        let run_id = events
            .first()
            .map(|e| e.run_id.clone())
            .unwrap_or_else(|| "unknown".to_string());

        Ok(Self {
            run_id,
            events,
            cursor: 0,
            matched_events: 0,
            divergences: Vec::new(),
        })
    }

    /// Constructs a replay engine from an in-memory sequence of events.
    pub fn new(run_id: impl Into<String>, events: Vec<Event>) -> Self {
        Self {
            run_id: run_id.into(),
            events,
            cursor: 0,
            matched_events: 0,
            divergences: Vec::new(),
        }
    }

    /// Returns the target run identifier being replayed.
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Current position (0-indexed) in the recorded event stream.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Total number of events in the recorded run.
    pub fn total_events(&self) -> usize {
        self.events.len()
    }

    /// Peeks at the next recorded event without advancing the cursor.
    pub fn peek_current(&self) -> Option<&Event> {
        self.events.get(self.cursor)
    }


    /// Verifies the next event in the stream matches the expected type and optional payload.
    /// Advances the cursor upon a successful match.
    pub fn verify_step(
        &mut self,
        expected_type: EventType,
        payload_to_verify: Option<&serde_json::Value>,
    ) -> Result<&Event, Divergence> {
        if self.cursor >= self.events.len() {
            let div = Divergence::LogExhausted {
                index: self.cursor + 1,
                attempted: format!("{expected_type:?}"),
            };
            self.divergences.push(div.clone());
            return Err(div);
        }

        let recorded = &self.events[self.cursor];

        if recorded.event_type != expected_type {
            let div = Divergence::TypeMismatch {
                index: self.cursor + 1,
                expected: expected_type,
                actual: recorded.event_type.clone(),
            };
            self.divergences.push(div.clone());
            return Err(div);
        }

        if let Some(expected_payload) = payload_to_verify {
            if &recorded.payload != expected_payload {
                let div = Divergence::PayloadMismatch {
                    index: self.cursor + 1,
                    expected: expected_payload.clone(),
                    actual: recorded.payload.clone(),
                };
                self.divergences.push(div.clone());
                return Err(div);
            }
        }

        self.cursor += 1;
        self.matched_events += 1;
        Ok(recorded)
    }

    /// Replays an LLM interaction:
    /// 1. Verifies that the next event is an `llm.request` matching the prompt payload.
    /// 2. Verifies that the subsequent event is an `llm.response`.
    /// 3. Returns the saved response payload WITHOUT making any external network requests.
    pub fn replay_llm(&mut self, request_payload: &serde_json::Value) -> Result<serde_json::Value, Divergence> {
        // Step 1: verify request
        self.verify_step(EventType::LlmRequest, Some(request_payload))?;

        // Step 2: verify and return response
        if self.cursor >= self.events.len() {
            let div = Divergence::MissingResponse {
                index: self.cursor + 1,
                expected_type: EventType::LlmResponse,
            };
            self.divergences.push(div.clone());
            return Err(div);
        }

        let resp_event = &self.events[self.cursor];
        if resp_event.event_type != EventType::LlmResponse {
            let div = Divergence::TypeMismatch {
                index: self.cursor + 1,
                expected: EventType::LlmResponse,
                actual: resp_event.event_type.clone(),
            };
            self.divergences.push(div.clone());
            return Err(div);
        }

        let payload = resp_event.payload.clone();
        self.cursor += 1;
        self.matched_events += 1;
        Ok(payload)
    }

    /// Replays a tool invocation:
    /// 1. Verifies that the next event is a `tool.started` matching the tool invocation.
    /// 2. Verifies that the subsequent event is `tool.completed` or `tool.failed`.
    /// 3. Returns the saved tool output WITHOUT executing any shell or filesystem commands.
    pub fn replay_tool(
        &mut self,
        tool_input_payload: &serde_json::Value,
    ) -> Result<(EventType, serde_json::Value), Divergence> {
        // Step 1: verify tool.started
        self.verify_step(EventType::ToolStarted, Some(tool_input_payload))?;

        // Step 2: retrieve tool.completed or tool.failed
        if self.cursor >= self.events.len() {
            let div = Divergence::MissingResponse {
                index: self.cursor + 1,
                expected_type: EventType::ToolCompleted,
            };
            self.divergences.push(div.clone());
            return Err(div);
        }

        let result_event = &self.events[self.cursor];
        match result_event.event_type {
            EventType::ToolCompleted | EventType::ToolFailed => {
                let ev_type = result_event.event_type.clone();
                let payload = result_event.payload.clone();
                self.cursor += 1;
                self.matched_events += 1;
                Ok((ev_type, payload))
            }
            _ => {
                let div = Divergence::TypeMismatch {
                    index: self.cursor + 1,
                    expected: EventType::ToolCompleted,
                    actual: result_event.event_type.clone(),
                };
                self.divergences.push(div.clone());
                Err(div)
            }
        }
    }

    /// Finalizes the replay session and produces the `ReplayReport`.
    /// Checks for any unconsumed events remaining in the log.
    pub fn finish(mut self) -> ReplayReport {
        if self.cursor < self.events.len() {
            let remaining = self.events.len() - self.cursor;
            self.divergences.push(Divergence::UnconsumedEvents {
                remaining_count: remaining,
                next_event_type: self.events[self.cursor].event_type.clone(),
            });
        }

        ReplayReport {
            run_id: self.run_id,
            total_recorded_events: self.events.len(),
            matched_events: self.matched_events,
            divergences: self.divergences,
        }
    }
}

/// A simple mock agent used to demonstrate live execution vs. replayed execution.
pub struct MockAgent {
    pub model: String,
}

impl MockAgent {
    pub fn new(model: impl Into<String>) -> Self {
        Self { model: model.into() }
    }

    /// Live execution with actual side effects recorded to Kairo.
    pub fn execute_live<F>(
        &self,
        run: &mut Run,
        prompt: &str,
        mut tool_fn: F,
    ) -> Result<String, crate::run::RunError>
    where
        F: FnMut(&str, &serde_json::Value) -> serde_json::Value,
    {
        // 1. Send LLM prompt
        run.record(
            EventType::LlmRequest,
            json!({"model": self.model, "prompt": prompt}),
        )?;

        // 2. Simulated LLM decides to call a calculator tool
        let tool_call = json!({"tool": "calculator", "args": {"expression": "2 + 2"}});
        run.record(EventType::LlmResponse, tool_call.clone())?;

        // 3. Tool execution (LIVE side effect happens here)
        run.record(EventType::ToolStarted, tool_call.clone())?;
        let tool_result = tool_fn("calculator", &tool_call);
        run.record(EventType::ToolCompleted, json!({"output": tool_result}))?;

        // 4. Final LLM synthesis
        run.record(
            EventType::LlmRequest,
            json!({"prompt": "Provide final answer"}),
        )?;
        run.record(
            EventType::LlmResponse,
            json!({"answer": "Result is 4"}),
        )?;

        // 5. Complete the run
        run.complete(json!({"status": "success"}))?;
        Ok("Result is 4".to_string())
    }

    /// Replay execution: ZERO external APIs contacted, ZERO tools executed.
    pub fn execute_replay(
        &self,
        replay: &mut ReplayEngine,
        prompt: &str,
    ) -> Result<String, Divergence> {
        // Step 0: verify run.started
        replay.verify_step(EventType::RunStarted, None)?;

        // Step 1: verify LLM request and receive saved response
        let tool_call = replay.replay_llm(&json!({"model": self.model, "prompt": prompt}))?;

        // Step 2: verify tool request and receive saved response WITHOUT executing tool
        let (_ev_type, _output) = replay.replay_tool(&tool_call)?;

        // Step 3: verify second LLM request and receive saved answer
        let final_resp = replay.replay_llm(&json!({"prompt": "Provide final answer"}))?;

        // Step 4: verify run.completed
        replay.verify_step(EventType::RunCompleted, None)?;

        Ok(final_resp["answer"].as_str().unwrap_or_default().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicBool, Ordering};
    use uuid::Uuid;

    fn temp_replay_file() -> std::path::PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("kairo_test_replay_{}.jsonl", Uuid::new_v4()));
        path
    }

    #[test]
    fn test_successful_replay() {
        let path = temp_replay_file();
        let agent = MockAgent::new("gpt-4");

        // 1. Record a live run
        let mut run = Run::start(&path, json!({"agent": "mock"})).unwrap();
        let live_ans = agent
            .execute_live(&mut run, "Calculate 2 + 2", |_tool, _args| json!(4))
            .unwrap();
        assert_eq!(live_ans, "Result is 4");

        // 2. Replay the run using ReplayEngine
        let mut replay = ReplayEngine::from_file(&path).unwrap();
        assert_eq!(replay.total_events(), 8);

        let replay_ans = agent.execute_replay(&mut replay, "Calculate 2 + 2").unwrap();
        assert_eq!(replay_ans, "Result is 4");

        let report = replay.finish();
        assert!(report.is_success());
        assert_eq!(report.matched_events, 8);
        assert_eq!(report.divergences.len(), 0);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_missing_recorded_response() {
        let run_id = "run-missing-resp";
        // Create events where an llm.request has NO matching llm.response (ends prematurely)
        let events = vec![
            Event::new(run_id, EventType::RunStarted, json!({})),
            Event::new(
                run_id,
                EventType::LlmRequest,
                json!({"model": "gpt-4", "prompt": "Hi"}),
            ),
        ];

        let mut replay = ReplayEngine::new(run_id, events);
        // Step through run.started first
        replay.verify_step(EventType::RunStarted, None).unwrap();

        let res = replay.replay_llm(&json!({"model": "gpt-4", "prompt": "Hi"}));

        match res {
            Err(Divergence::MissingResponse { expected_type, .. }) => {
                assert_eq!(expected_type, EventType::LlmResponse);
            }
            other => panic!("Expected MissingResponse, got {other:?}"),
        }
    }

    #[test]
    fn test_event_sequence_mismatch() {
        let run_id = "run-seq-mismatch";
        let events = vec![
            Event::new(run_id, EventType::RunStarted, json!({})),
            // The log has a tool.started event instead of an llm.request
            Event::new(run_id, EventType::ToolStarted, json!({"tool": "bash"})),
        ];

        let mut replay = ReplayEngine::new(run_id, events);
        // Step through run.started first
        replay.verify_step(EventType::RunStarted, None).unwrap();

        // Consumer tries to send an LLM request, but next recorded is tool.started
        let res = replay.replay_llm(&json!({"prompt": "Hello"}));

        match res {
            Err(Divergence::TypeMismatch { expected, actual, .. }) => {
                assert_eq!(expected, EventType::LlmRequest);
                assert_eq!(actual, EventType::ToolStarted);
            }
            other => panic!("Expected TypeMismatch, got {other:?}"),
        }
    }


    #[test]
    fn test_different_tool_output_or_input() {
        let run_id = "run-tool-diff";
        let events = vec![
            Event::new(
                run_id,
                EventType::ToolStarted,
                json!({"tool": "search", "query": "rust lang"}),
            ),
            Event::new(
                run_id,
                EventType::ToolCompleted,
                json!({"output": "found 100 results"}),
            ),
        ];

        let mut replay = ReplayEngine::new(run_id, events);
        // Consumer submits a DIFFERENT query than what was recorded
        let res = replay.replay_tool(&json!({"tool": "search", "query": "python lang"}));

        match res {
            Err(Divergence::PayloadMismatch { expected, actual, .. }) => {
                assert_eq!(expected["query"], "python lang");
                assert_eq!(actual["query"], "rust lang");
            }
            other => panic!("Expected PayloadMismatch, got {other:?}"),
        }
    }

    #[test]
    fn test_ensuring_no_external_api_or_shell_command_executed() {
        let path = temp_replay_file();
        let agent = MockAgent::new("gpt-4");

        // Flag to track whether external side effects occurred
        static SIDE_EFFECT_CALLED: AtomicBool = AtomicBool::new(false);

        // 1. Live run triggers side effect
        let mut run = Run::start(&path, json!({})).unwrap();
        agent
            .execute_live(&mut run, "Test prompt", |_tool, _args| {
                SIDE_EFFECT_CALLED.store(true, Ordering::SeqCst);
                json!({"status": "executed"})
            })
            .unwrap();

        assert!(SIDE_EFFECT_CALLED.load(Ordering::SeqCst));

        // Reset the flag to false before replay
        SIDE_EFFECT_CALLED.store(false, Ordering::SeqCst);

        // 2. Replay execution
        let mut replay = ReplayEngine::from_file(&path).unwrap();
        let res = agent.execute_replay(&mut replay, "Test prompt");
        assert!(res.is_ok());

        // PROOF: The side effect flag is STILL false during replay!
        assert_eq!(
            SIDE_EFFECT_CALLED.load(Ordering::SeqCst),
            false,
            "Side effect must never be executed during replay!"
        );

        let _ = fs::remove_file(path);
    }
}
