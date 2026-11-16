use crate::event::{current_utc_iso8601, Event, EventType};
use crate::run::{Run, RunStatus};
use crate::storage::{JsonlStorage, StorageError};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Metadata representing an execution checkpoint pinned to a specific event in a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checkpoint {
    pub checkpoint_id: String,
    pub run_id: String,
    /// 1-based index of the event in the run's history.
    pub event_index: usize,
    /// Unique identifier of the event at this position.
    pub event_id: String,
    pub name: String,
    pub created_at: String,
    pub metadata: serde_json::Value,
}

/// Errors relating to checkpoint creation, retrieval, and branching.
#[derive(Debug)]
pub enum CheckpointError {
    RunNotFound(String),
    EventIndexOutOfBounds { index: usize, total_events: usize },
    CheckpointNotFound(String),
    Storage(StorageError),
    Io(std::io::Error),
    InvalidCheckpointFile(String),
    CorruptedLineage(String),
}

impl std::fmt::Display for CheckpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckpointError::RunNotFound(id) => write!(f, "Run '{id}' not found"),
            CheckpointError::EventIndexOutOfBounds { index, total_events } => {
                write!(
                    f,
                    "Checkpoint step {index} is out of bounds (run has {total_events} events)"
                )
            }
            CheckpointError::CheckpointNotFound(id) => write!(f, "Checkpoint '{id}' not found"),
            CheckpointError::Storage(err) => write!(f, "Storage error: {err}"),
            CheckpointError::Io(err) => write!(f, "I/O error: {err}"),
            CheckpointError::InvalidCheckpointFile(msg) => write!(f, "Invalid checkpoint file: {msg}"),
            CheckpointError::CorruptedLineage(msg) => write!(f, "Corrupted lineage: {msg}"),
        }
    }
}

impl std::error::Error for CheckpointError {}

impl From<StorageError> for CheckpointError {
    fn from(err: StorageError) -> Self {
        CheckpointError::Storage(err)
    }
}

impl From<std::io::Error> for CheckpointError {
    fn from(err: std::io::Error) -> Self {
        CheckpointError::Io(err)
    }
}

/// Represents a single node in the execution lineage tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineageNode {
    pub run_id: String,
    pub status: RunStatus,
    pub parent_run_id: Option<String>,
    pub checkpoint_id: Option<String>,
    pub fork_event_index: Option<usize>,
    pub event_count: usize,
}

/// Full ancestral lineage chain for a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineageReport {
    pub target_run_id: String,
    pub chain: Vec<LineageNode>,
    pub total_effective_events: usize,
}

/// Checkpoint manager handling creation, storage, and branching.
pub struct CheckpointManager;

impl CheckpointManager {
    /// Returns the directory where checkpoints are stored (<dir>/checkpoints).
    pub fn checkpoints_dir(base_dir: impl AsRef<Path>) -> PathBuf {
        base_dir.as_ref().join("checkpoints")
    }

    /// Creates a checkpoint at a specific 1-based event position in an existing run.
    pub fn create_checkpoint(
        base_dir: impl AsRef<Path>,
        run_id: &str,
        event_step_1_based: usize,
        name: impl Into<String>,
        metadata: serde_json::Value,
    ) -> Result<Checkpoint, CheckpointError> {
        let base_dir = base_dir.as_ref();

        // 1. Locate and read the run
        let run_path = Run::find_path(base_dir, run_id)
            .ok_or_else(|| CheckpointError::RunNotFound(run_id.to_string()))?;

        let events = JsonlStorage::read_events(&run_path)?;
        if event_step_1_based == 0 || event_step_1_based > events.len() {
            return Err(CheckpointError::EventIndexOutOfBounds {
                index: event_step_1_based,
                total_events: events.len(),
            });
        }

        let target_event = &events[event_step_1_based - 1];
        let checkpoint_id = format!("cp-{}", &Uuid::new_v4().to_string()[..8]);

        let checkpoint = Checkpoint {
            checkpoint_id: checkpoint_id.clone(),
            run_id: run_id.to_string(),
            event_index: event_step_1_based,
            event_id: target_event.event_id.clone(),
            name: name.into(),
            created_at: current_utc_iso8601(),
            metadata,
        };

        // 2. Persist checkpoint metadata to <base_dir>/checkpoints/<checkpoint_id>.json
        let cp_dir = Self::checkpoints_dir(base_dir);
        fs::create_dir_all(&cp_dir)?;

        let cp_file = cp_dir.join(format!("{checkpoint_id}.json"));
        let json_str = serde_json::to_string_pretty(&checkpoint)
            .map_err(|e| CheckpointError::InvalidCheckpointFile(e.to_string()))?;
        fs::write(cp_file, json_str)?;

        Ok(checkpoint)
    }

    /// Loads a checkpoint by its ID.
    pub fn load_checkpoint(
        base_dir: impl AsRef<Path>,
        checkpoint_id: &str,
    ) -> Result<Checkpoint, CheckpointError> {
        let cp_file = Self::checkpoints_dir(base_dir).join(format!("{checkpoint_id}.json"));
        if !cp_file.exists() {
            return Err(CheckpointError::CheckpointNotFound(checkpoint_id.to_string()));
        }

        let content = fs::read_to_string(cp_file)?;
        let checkpoint: Checkpoint = serde_json::from_str(&content)
            .map_err(|e| CheckpointError::InvalidCheckpointFile(e.to_string()))?;

        Ok(checkpoint)
    }

    /// Lists all checkpoints stored in the data directory, optionally filtered by run ID.
    pub fn list_checkpoints(
        base_dir: impl AsRef<Path>,
        filter_run_id: Option<&str>,
    ) -> Result<Vec<Checkpoint>, CheckpointError> {
        let cp_dir = Self::checkpoints_dir(base_dir);
        if !cp_dir.exists() {
            return Ok(Vec::new());
        }

        let mut list = Vec::new();
        for entry in fs::read_dir(cp_dir)?.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(cp) = serde_json::from_str::<Checkpoint>(&content) {
                        if let Some(target_run) = filter_run_id {
                            if cp.run_id == target_run {
                                list.push(cp);
                            }
                        } else {
                            list.push(cp);
                        }
                    }
                }
            }
        }

        list.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(list)
    }

    /// Creates a new branch run based on an existing checkpoint.
    ///
    /// The parent run file is NEVER modified or copied. The new run stores
    /// lineage metadata pointing back to its parent and checkpoint.
    pub fn branch_from_checkpoint(
        base_dir: impl AsRef<Path>,
        checkpoint_id: &str,
        new_run_id: Option<String>,
        branch_metadata: serde_json::Value,
    ) -> Result<Run, CheckpointError> {
        let base_dir = base_dir.as_ref();
        let checkpoint = Self::load_checkpoint(base_dir, checkpoint_id)?;

        // Verify parent run still exists
        let _parent_path = Run::find_path(base_dir, &checkpoint.run_id)
            .ok_or_else(|| CheckpointError::RunNotFound(checkpoint.run_id.clone()))?;

        let run_id = new_run_id.unwrap_or_else(|| format!("run-branch-{}", &Uuid::new_v4().to_string()[..8]));
        let run_file = base_dir.join(format!("{run_id}.jsonl"));

        let start_payload = json!({
            "branch": true,
            "parent_run_id": checkpoint.run_id,
            "parent_checkpoint_id": checkpoint.checkpoint_id,
            "parent_event_index": checkpoint.event_index,
            "parent_event_id": checkpoint.event_id,
            "branch_metadata": branch_metadata,
        });

        let branch_run = Run::start_with_id(&run_id, &run_file, start_payload)
            .map_err(|e| CheckpointError::Storage(StorageError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))))?;

        Ok(branch_run)
    }

    /// Reconstructs the complete effective event history of a run by walking
    /// backwards through its parent lineage pointers without copying past data.
    pub fn reconstruct_history(
        base_dir: impl AsRef<Path>,
        run_id: &str,
    ) -> Result<Vec<Event>, CheckpointError> {
        let base_dir = base_dir.as_ref();
        let run_path = Run::find_path(base_dir, run_id)
            .ok_or_else(|| CheckpointError::RunNotFound(run_id.to_string()))?;

        let events = JsonlStorage::read_events(&run_path)?;
        if events.is_empty() {
            return Ok(Vec::new());
        }

        let first = &events[0];
        if first.event_type != EventType::RunStarted {
            return Err(CheckpointError::CorruptedLineage("Run log does not begin with run.started".to_string()));
        }

        // Check if this run is a branch
        if let Some(parent_run_id) = first.payload.get("parent_run_id").and_then(|v| v.as_str()) {
            let parent_event_index = first
                .payload
                .get("parent_event_index")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;

            // Recursively reconstruct parent history
            let parent_history = Self::reconstruct_history(base_dir, parent_run_id)?;

            if parent_event_index > parent_history.len() {
                return Err(CheckpointError::CorruptedLineage(format!(
                    "Parent event index {parent_event_index} exceeds parent history length {}",
                    parent_history.len()
                )));
            }

            // Slice ancestor events up to the checkpoint position
            let mut combined = parent_history[..parent_event_index].to_vec();

            // Append the branch's own events (skipping its synthetic branch run.started envelope)
            for branch_ev in events.iter().skip(1) {
                combined.push(branch_ev.clone());
            }

            Ok(combined)
        } else {
            // Root run (not a branch)
            Ok(events)
        }
    }

    /// Inspects the ancestral lineage chain from the root run down to the target run.
    pub fn inspect_lineage(
        base_dir: impl AsRef<Path>,
        run_id: &str,
    ) -> Result<LineageReport, CheckpointError> {
        let base_dir = base_dir.as_ref();
        let mut chain = Vec::new();
        let mut current_id = run_id.to_string();

        let effective_history = Self::reconstruct_history(base_dir, run_id)?;

        loop {
            let path = Run::find_path(base_dir, &current_id)
                .ok_or_else(|| CheckpointError::RunNotFound(current_id.clone()))?;

            let run = Run::load(&path)
                .map_err(|e| CheckpointError::Storage(StorageError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))))?;

            let events = JsonlStorage::read_events(&path)?;
            let first = events.first();

            let parent_run_id = first
                .and_then(|e| e.payload.get("parent_run_id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let checkpoint_id = first
                .and_then(|e| e.payload.get("parent_checkpoint_id"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());

            let fork_event_index = first
                .and_then(|e| e.payload.get("parent_event_index"))
                .and_then(|v| v.as_u64())
                .map(|n| n as usize);

            chain.push(LineageNode {
                run_id: current_id.clone(),
                status: run.status(),
                parent_run_id: parent_run_id.clone(),
                checkpoint_id,
                fork_event_index,
                event_count: events.len(),
            });

            if let Some(parent) = parent_run_id {
                current_id = parent;
            } else {
                break;
            }
        }

        chain.reverse(); // Order from root run -> descendants -> target run

        Ok(LineageReport {
            target_run_id: run_id.to_string(),
            chain,
            total_effective_events: effective_history.len(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_test_dir(label: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("kairo_test_cp_{}_{}", label, &Uuid::new_v4().to_string()[..8]));
        let _ = fs::create_dir_all(&path);
        path
    }

    #[test]
    fn test_creating_a_checkpoint() {
        let dir = temp_test_dir("create");
        let run_file = dir.join("run-origin.jsonl");

        let mut run = Run::start_with_id("run-origin", &run_file, json!({"agent": "lead"})).unwrap();
        run.record(EventType::ToolStarted, json!({"step": 1})).unwrap();
        run.record(EventType::ToolCompleted, json!({"step": 1, "status": "ok"})).unwrap();
        run.complete(json!({"summary": "done"})).unwrap();

        // Checkpoint at step 2 (ToolStarted)
        let cp = CheckpointManager::create_checkpoint(
            &dir,
            "run-origin",
            2,
            "before-tool-completed",
            json!({"note": "good state"}),
        )
        .expect("Failed to create checkpoint");

        assert_eq!(cp.run_id, "run-origin");
        assert_eq!(cp.event_index, 2);
        assert_eq!(cp.name, "before-tool-completed");
        assert!(cp.checkpoint_id.starts_with("cp-"));

        // Verify it can be loaded
        let loaded = CheckpointManager::load_checkpoint(&dir, &cp.checkpoint_id).unwrap();
        assert_eq!(loaded, cp);

        // Verify listing
        let list = CheckpointManager::list_checkpoints(&dir, None).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0], cp);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_branching_from_a_checkpoint() {
        let dir = temp_test_dir("branch");
        let run_file = dir.join("run-main.jsonl");

        let mut run = Run::start_with_id("run-main", &run_file, json!({})).unwrap();
        run.record(EventType::ToolStarted, json!({"cmd": "git checkout"})).unwrap();
        run.complete(json!({})).unwrap();

        let cp = CheckpointManager::create_checkpoint(&dir, "run-main", 2, "at-checkout", json!({})).unwrap();

        // Create a branch from checkpoint
        let mut branch = CheckpointManager::branch_from_checkpoint(
            &dir,
            &cp.checkpoint_id,
            Some("run-branch-exp".to_string()),
            json!({"reason": "try alternative path"}),
        )
        .expect("Failed to branch");

        assert_eq!(branch.run_id(), "run-branch-exp");
        assert_eq!(branch.status(), RunStatus::Running);

        // Record a new event on the branch
        branch.record(EventType::ToolStarted, json!({"cmd": "git rebase"})).unwrap();
        branch.complete(json!({"outcome": "alternative success"})).unwrap();

        assert_eq!(branch.status(), RunStatus::Completed);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_inspecting_branch_history_reconstruction() {
        let dir = temp_test_dir("reconstruct");
        let run_file = dir.join("run-root.jsonl");

        // 1. Root run: 3 events
        let mut root = Run::start_with_id("run-root", &run_file, json!({"init": true})).unwrap();
        root.record(EventType::ToolStarted, json!({"tool": "search"})).unwrap();
        root.complete(json!({"root": "done"})).unwrap();

        // 2. Checkpoint at step 2 (ToolStarted)
        let cp = CheckpointManager::create_checkpoint(&dir, "run-root", 2, "after-search", json!({})).unwrap();

        // 3. Branch from checkpoint and add 2 new events
        let mut branch = CheckpointManager::branch_from_checkpoint(
            &dir,
            &cp.checkpoint_id,
            Some("run-sub".to_string()),
            json!({}),
        )
        .unwrap();

        branch.record(EventType::ToolStarted, json!({"tool": "calculator"})).unwrap();
        branch.complete(json!({"branch": "done"})).unwrap();

        // 4. Reconstruct effective history for the branch:
        // Expected:
        // - Event 1: run.started from run-root
        // - Event 2: tool.started from run-root
        // - Event 3: tool.started from run-sub
        // - Event 4: run.completed from run-sub
        let effective = CheckpointManager::reconstruct_history(&dir, "run-sub").unwrap();
        assert_eq!(effective.len(), 4);
        assert_eq!(effective[0].run_id, "run-root");
        assert_eq!(effective[1].run_id, "run-root");
        assert_eq!(effective[1].event_type, EventType::ToolStarted);
        assert_eq!(effective[2].run_id, "run-sub");
        assert_eq!(effective[2].payload["tool"], "calculator");
        assert_eq!(effective[3].run_id, "run-sub");
        assert_eq!(effective[3].event_type, EventType::RunCompleted);

        // 5. Inspect lineage report
        let lineage = CheckpointManager::inspect_lineage(&dir, "run-sub").unwrap();
        assert_eq!(lineage.chain.len(), 2);
        assert_eq!(lineage.chain[0].run_id, "run-root");
        assert_eq!(lineage.chain[1].run_id, "run-sub");
        assert_eq!(lineage.chain[1].checkpoint_id, Some(cp.checkpoint_id));
        assert_eq!(lineage.total_effective_events, 4);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_preserving_original_run() {
        let dir = temp_test_dir("preserve");
        let run_file = dir.join("run-immut.jsonl");

        let mut root = Run::start_with_id("run-immut", &run_file, json!({})).unwrap();
        root.record(EventType::ToolStarted, json!({"step": 1})).unwrap();
        root.complete(json!({"status": "original_done"})).unwrap();

        let original_content = fs::read_to_string(&run_file).unwrap();
        let original_event_count = JsonlStorage::read_events(&run_file).unwrap().len();

        let cp = CheckpointManager::create_checkpoint(&dir, "run-immut", 2, "freeze-point", json!({})).unwrap();

        // Create branch and do lots of work on the branch
        let mut branch = CheckpointManager::branch_from_checkpoint(&dir, &cp.checkpoint_id, None, json!({})).unwrap();
        branch.record(EventType::ToolStarted, json!({"step": "branch_1"})).unwrap();
        branch.record(EventType::ToolCompleted, json!({"step": "branch_1"})).unwrap();
        branch.complete(json!({})).unwrap();

        // VERIFY: Original run file is 100% UNCHANGED
        let current_content = fs::read_to_string(&run_file).unwrap();
        let current_events = JsonlStorage::read_events(&run_file).unwrap();

        assert_eq!(original_content, current_content, "Original run file was mutated!");
        assert_eq!(original_event_count, current_events.len(), "Original event count changed!");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_invalid_checkpoint_references() {
        let dir = temp_test_dir("invalid");
        let run_file = dir.join("run-small.jsonl");

        let mut run = Run::start_with_id("run-small", &run_file, json!({})).unwrap();
        run.complete(json!({})).unwrap(); // 2 events total

        // 1. Invalid step index (step 0 or step 99)
        let err_0 = CheckpointManager::create_checkpoint(&dir, "run-small", 0, "bad-0", json!({}));
        assert!(matches!(err_0, Err(CheckpointError::EventIndexOutOfBounds { .. })));

        let err_99 = CheckpointManager::create_checkpoint(&dir, "run-small", 99, "bad-99", json!({}));
        assert!(matches!(err_99, Err(CheckpointError::EventIndexOutOfBounds { .. })));

        // 2. Checkpoint on nonexistent run
        let err_run = CheckpointManager::create_checkpoint(&dir, "run-ghost", 1, "ghost", json!({}));
        assert!(matches!(err_run, Err(CheckpointError::RunNotFound(..))));

        // 3. Branching from nonexistent checkpoint
        let err_cp = CheckpointManager::branch_from_checkpoint(&dir, "cp-nonexistent-123", None, json!({}));
        assert!(matches!(err_cp, Err(CheckpointError::CheckpointNotFound(..))));

        let _ = fs::remove_dir_all(dir);
    }
}
