use crate::event::{Event, EventType};
use crate::storage::{JsonlStorage, StorageError};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Lifecycle status of an agent execution run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Completed,
    Failed,
}

impl std::fmt::Display for RunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunStatus::Running => write!(f, "running"),
            RunStatus::Completed => write!(f, "completed"),
            RunStatus::Failed => write!(f, "failed"),
        }
    }
}

/// Errors that can occur during run lifecycle operations or event recording.
#[derive(Debug)]
pub enum RunError {
    /// Attempted a lifecycle transition or action not permitted in the current state.
    InvalidStateTransition {
        current: RunStatus,
        attempted: &'static str,
    },
    /// A submitted event has a different run_id than this run.
    MismatchedRunId {
        expected: String,
        found: String,
    },
    /// Underlying storage failure.
    Storage(StorageError),
    /// Run log was empty or lacked a run.started event upon load.
    InvalidRunLog(String),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::InvalidStateTransition { current, attempted } => {
                write!(
                    f,
                    "Cannot perform '{attempted}' on run currently in '{current}' state"
                )
            }
            RunError::MismatchedRunId { expected, found } => {
                write!(
                    f,
                    "Submitted event run_id '{found}' does not match run '{expected}'"
                )
            }
            RunError::Storage(err) => write!(f, "Storage error: {err}"),
            RunError::InvalidRunLog(msg) => write!(f, "Invalid run log: {msg}"),
        }
    }
}

impl std::error::Error for RunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RunError::Storage(err) => Some(err),
            _ => None,
        }
    }
}

impl From<StorageError> for RunError {
    fn from(err: StorageError) -> Self {
        RunError::Storage(err)
    }
}

/// Manages the lifecycle and event recording for an individual execution run.
pub struct Run {
    run_id: String,
    status: RunStatus,
    storage_path: PathBuf,
}

impl Run {
    /// Starts a new run with an auto-generated UUIDv4 identifier.
    /// Emits and persists the initial `run.started` event immediately.
    pub fn start(storage_path: impl Into<PathBuf>, payload: serde_json::Value) -> Result<Self, RunError> {
        let run_id = format!("run-{}", Uuid::new_v4());
        Self::start_with_id(run_id, storage_path, payload)
    }

    /// Starts a new run with an explicit caller-provided run identifier.
    /// Emits and persists the initial `run.started` event immediately.
    pub fn start_with_id(
        run_id: impl Into<String>,
        storage_path: impl Into<PathBuf>,
        payload: serde_json::Value,
    ) -> Result<Self, RunError> {
        let run_id = run_id.into();
        let storage_path = storage_path.into();

        let start_event = Event::new(&run_id, EventType::RunStarted, payload);
        JsonlStorage::append_event(&storage_path, &start_event)?;

        Ok(Self {
            run_id,
            status: RunStatus::Running,
            storage_path,
        })
    }

    /// Reconstructs a run from an existing JSONL event log file.
    pub fn load(storage_path: impl Into<PathBuf>) -> Result<Self, RunError> {
        let storage_path = storage_path.into();
        let events = JsonlStorage::read_events(&storage_path)?;

        if events.is_empty() {
            return Err(RunError::InvalidRunLog("Run event log is empty".to_string()));
        }

        let first = &events[0];
        if first.event_type != EventType::RunStarted {
            return Err(RunError::InvalidRunLog(
                "First event must be 'run.started'".to_string(),
            ));
        }

        let run_id = first.run_id.clone();
        let mut status = RunStatus::Running;

        for event in events.iter().skip(1) {
            match event.event_type {
                EventType::RunCompleted => status = RunStatus::Completed,
                EventType::RunFailed => status = RunStatus::Failed,
                _ => {}
            }
        }

        Ok(Self {
            run_id,
            status,
            storage_path,
        })
    }

    /// Returns the unique run identifier.
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Returns the current lifecycle status of the run.
    pub fn status(&self) -> RunStatus {
        self.status
    }

    /// Returns the file path where events are persisted.
    pub fn storage_path(&self) -> &Path {
        &self.storage_path
    }

    /// Records an event with the specified type and payload for this run.
    /// Fails if the run is not in the `Running` state.
    pub fn record(&self, event_type: EventType, payload: serde_json::Value) -> Result<Event, RunError> {
        if self.status != RunStatus::Running {
            return Err(RunError::InvalidStateTransition {
                current: self.status,
                attempted: "record",
            });
        }

        let event = Event::new(&self.run_id, event_type, payload);
        JsonlStorage::append_event(&self.storage_path, &event)?;
        Ok(event)
    }

    /// Submits a pre-constructed `Event` to be recorded for this run.
    ///
    /// Validates that:
    /// 1. The run is in the `Running` state.
    /// 2. The event's `run_id` strictly matches this run's ID.
    pub fn record_event(&self, event: &Event) -> Result<(), RunError> {
        if self.status != RunStatus::Running {
            return Err(RunError::InvalidStateTransition {
                current: self.status,
                attempted: "record_event",
            });
        }

        if event.run_id != self.run_id {
            return Err(RunError::MismatchedRunId {
                expected: self.run_id.clone(),
                found: event.run_id.clone(),
            });
        }

        JsonlStorage::append_event(&self.storage_path, event)?;
        Ok(())
    }

    /// Marks the run as successfully completed.
    ///
    /// Emits and persists a `run.completed` event.
    /// Fails if the run is not in the `Running` state.
    pub fn complete(&mut self, payload: serde_json::Value) -> Result<Event, RunError> {
        if self.status != RunStatus::Running {
            return Err(RunError::InvalidStateTransition {
                current: self.status,
                attempted: "complete",
            });
        }

        let event = Event::new(&self.run_id, EventType::RunCompleted, payload);
        JsonlStorage::append_event(&self.storage_path, &event)?;
        self.status = RunStatus::Completed;
        Ok(event)
    }

    /// Marks the run as failed.
    ///
    /// Emits and persists a `run.failed` event.
    /// Fails if the run is not in the `Running` state.
    pub fn fail(&mut self, payload: serde_json::Value) -> Result<Event, RunError> {
        if self.status != RunStatus::Running {
            return Err(RunError::InvalidStateTransition {
                current: self.status,
                attempted: "fail",
            });
        }

        let event = Event::new(&self.run_id, EventType::RunFailed, payload);
        JsonlStorage::append_event(&self.storage_path, &event)?;
        self.status = RunStatus::Failed;
        Ok(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    fn temp_run_file(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let unique = Uuid::new_v4().to_string();
        path.push(format!("kairo_test_run_{name}_{unique}.jsonl"));
        path
    }

    #[test]
    fn test_starting_a_run() {
        let path = temp_run_file("start");
        let run = Run::start(&path, json!({"agent": "tester"})).expect("Failed to start run");

        assert_eq!(run.status(), RunStatus::Running);
        assert!(run.run_id().starts_with("run-"));

        // Verify that run.started was persisted to storage
        let events = JsonlStorage::read_events(&path).expect("Failed to read storage");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, EventType::RunStarted);
        assert_eq!(events[0].run_id, run.run_id());
        assert_eq!(events[0].payload["agent"], "tester");

        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_recording_events() {
        let path = temp_run_file("record");
        let run = Run::start(&path, json!({})).unwrap();

        let ev1 = run
            .record(EventType::ToolStarted, json!({"tool": "calculator"}))
            .unwrap();
        let ev2 = run
            .record(EventType::ToolCompleted, json!({"result": 42}))
            .unwrap();

        assert_eq!(ev1.run_id, run.run_id());
        assert_eq!(ev2.run_id, run.run_id());

        let events = JsonlStorage::read_events(&path).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[1].event_type, EventType::ToolStarted);
        assert_eq!(events[2].event_type, EventType::ToolCompleted);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_completing_a_run() {
        let path = temp_run_file("complete");
        let mut run = Run::start(&path, json!({})).unwrap();

        run.record(EventType::ToolCompleted, json!({})).unwrap();
        let comp_event = run.complete(json!({"summary": "done successfully"})).unwrap();

        assert_eq!(run.status(), RunStatus::Completed);
        assert_eq!(comp_event.event_type, EventType::RunCompleted);

        // Verify persisted events
        let events = JsonlStorage::read_events(&path).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[2].event_type, EventType::RunCompleted);

        // Verify loading back from disk reflects Completed status
        let loaded = Run::load(&path).unwrap();
        assert_eq!(loaded.status(), RunStatus::Completed);
        assert_eq!(loaded.run_id(), run.run_id());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_failing_a_run() {
        let path = temp_run_file("fail");
        let mut run = Run::start(&path, json!({})).unwrap();

        let fail_event = run
            .fail(json!({"error": "network timeout", "code": 504}))
            .unwrap();

        assert_eq!(run.status(), RunStatus::Failed);
        assert_eq!(fail_event.event_type, EventType::RunFailed);

        let events = JsonlStorage::read_events(&path).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].event_type, EventType::RunFailed);

        let loaded = Run::load(&path).unwrap();
        assert_eq!(loaded.status(), RunStatus::Failed);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_rejecting_invalid_lifecycle_operations() {
        let path = temp_run_file("invalid_ops");
        let mut run = Run::start(&path, json!({})).unwrap();

        run.complete(json!({"reason": "first completion"})).unwrap();
        assert_eq!(run.status(), RunStatus::Completed);

        // 1. Cannot complete an already completed run
        let second_comp = run.complete(json!({}));
        match second_comp {
            Err(RunError::InvalidStateTransition { current, attempted }) => {
                assert_eq!(current, RunStatus::Completed);
                assert_eq!(attempted, "complete");
            }
            other => panic!("Expected InvalidStateTransition on double complete, got {other:?}"),
        }

        // 2. Cannot fail an already completed run
        let fail_after_complete = run.fail(json!({}));
        assert!(matches!(
            fail_after_complete,
            Err(RunError::InvalidStateTransition { .. })
        ));

        // 3. Cannot record new events on a completed run
        let record_after_complete = run.record(EventType::ToolStarted, json!({}));
        assert!(matches!(
            record_after_complete,
            Err(RunError::InvalidStateTransition { .. })
        ));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn test_checking_events_belong_to_correct_run() {
        let path = temp_run_file("mismatched_id");
        let run = Run::start_with_id("run-target-123", &path, json!({})).unwrap();

        // Submitting an event with the matching run_id succeeds
        let valid_event = Event::new("run-target-123", EventType::ToolStarted, json!({}));
        assert!(run.record_event(&valid_event).is_ok());

        // Submitting an event with a foreign/alien run_id fails
        let foreign_event = Event::new("run-alien-999", EventType::ToolStarted, json!({}));
        let res = run.record_event(&foreign_event);

        match res {
            Err(RunError::MismatchedRunId { expected, found }) => {
                assert_eq!(expected, "run-target-123");
                assert_eq!(found, "run-alien-999");
            }
            other => panic!("Expected MismatchedRunId error, got {other:?}"),
        }

        let _ = fs::remove_file(path);
    }
}

