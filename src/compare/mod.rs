use crate::event::{Event, EventType};
use crate::run::Run;
use crate::storage::JsonlStorage;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// The semantic classification of a difference between two runs at an event step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffKind {
    /// Both runs performed the same event with matching payload.
    Identical,
    /// Event type matched, but payload content differed.
    Modified {
        field_differences: Vec<String>,
    },
    /// Event exists in Run A but was omitted in Run B.
    Removed,
    /// Event was added in Run B and did not exist in Run A.
    Added,
}

impl std::fmt::Display for DiffKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiffKind::Identical => write!(f, "IDENTICAL"),
            DiffKind::Modified { field_differences } => {
                write!(f, "MODIFIED ({})", field_differences.join(", "))
            }
            DiffKind::Removed => write!(f, "REMOVED (only in A)"),
            DiffKind::Added => write!(f, "ADDED (only in B)"),
        }
    }
}

/// A comparative step aligning events from Run A and Run B.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiffItem {
    pub step_a: Option<usize>,
    pub step_b: Option<usize>,
    pub kind: DiffKind,
    pub event_a: Option<Event>,
    pub event_b: Option<Event>,
}

/// Description of the first point where Run A and Run B diverged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirstDivergence {
    /// 1-based step index where divergence occurred.
    pub step: usize,
    /// Explanation of why the events differed.
    pub reason: String,
    pub event_a_type: Option<EventType>,
    pub event_b_type: Option<EventType>,
}

/// Detailed comparison report between two runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiffReport {

    pub run_id_a: String,
    pub run_id_b: String,
    pub total_events_a: usize,
    pub total_events_b: usize,
    pub is_identical: bool,
    pub first_divergence: Option<FirstDivergence>,
    pub identical_count: usize,
    pub modified_count: usize,
    pub added_count: usize,
    pub removed_count: usize,
    pub items: Vec<DiffItem>,
}

/// Determines whether two events are semantically equal.
///
/// COMPARISON STRATEGY:
/// - Evaluates `event_type` and `payload`.
/// - Generated volatile fields (`event_id`, `run_id`, `timestamp`) are intentionally IGNORED.
///   Every run naturally gets fresh UUIDs and timestamps; execution behavior is identical
///   if the agent did the exact same operations with the exact same inputs and outputs.
pub fn events_semantically_equal(a: &Event, b: &Event) -> bool {
    a.event_type == b.event_type && a.payload == b.payload
}

/// Computes field-level differences between two JSON payloads.
pub fn diff_payloads(a: &serde_json::Value, b: &serde_json::Value) -> Vec<String> {
    let mut diffs = Vec::new();
    match (a, b) {
        (serde_json::Value::Object(map_a), serde_json::Value::Object(map_b)) => {
            for (k, val_a) in map_a {
                match map_b.get(k) {
                    Some(val_b) => {
                        if val_a != val_b {
                            diffs.push(format!("{k}: '{val_a}' != '{val_b}'"));
                        }
                    }
                    None => {
                        diffs.push(format!("field '{k}' missing in B"));
                    }
                }
            }
            for k in map_b.keys() {
                if !map_a.contains_key(k) {
                    diffs.push(format!("field '{k}' only in B"));
                }
            }
        }
        _ => {
            if a != b {
                diffs.push(format!("'{a}' != '{b}'"));
            }
        }
    }
    diffs
}

/// Identifies the earliest event index where Run A and Run B differ.
pub fn find_first_divergence(events_a: &[Event], events_b: &[Event]) -> Option<FirstDivergence> {
    let min_len = events_a.len().min(events_b.len());
    for i in 0..min_len {
        let ea = &events_a[i];
        let eb = &events_b[i];

        if ea.event_type != eb.event_type {
            return Some(FirstDivergence {
                step: i + 1,
                reason: format!(
                    "Event type differs: expected {:?}, got {:?}",
                    ea.event_type, eb.event_type
                ),
                event_a_type: Some(ea.event_type.clone()),
                event_b_type: Some(eb.event_type.clone()),
            });
        }

        if ea.payload != eb.payload {
            let field_diffs = diff_payloads(&ea.payload, &eb.payload);
            let detail = if field_diffs.is_empty() {
                "Payload values differ".to_string()
            } else {
                field_diffs.join(", ")
            };
            return Some(FirstDivergence {
                step: i + 1,
                reason: format!("Payload differs: {detail}"),
                event_a_type: Some(ea.event_type.clone()),
                event_b_type: Some(eb.event_type.clone()),
            });
        }
    }

    if events_a.len() != events_b.len() {
        if events_a.len() > events_b.len() {
            let step = min_len + 1;
            let next_type = events_a[min_len].event_type.clone();
            return Some(FirstDivergence {
                step,
                reason: format!(
                    "Run B terminated early at step {min_len}; Run A continued with {:?}",
                    next_type
                ),
                event_a_type: Some(next_type),
                event_b_type: None,
            });
        } else {
            let step = min_len + 1;
            let next_type = events_b[min_len].event_type.clone();
            return Some(FirstDivergence {
                step,
                reason: format!(
                    "Run A terminated early at step {min_len}; Run B continued with {:?}",
                    next_type
                ),
                event_a_type: None,
                event_b_type: Some(next_type),
            });
        }
    }

    None
}

/// Compares two run histories using a Longest Common Subsequence (LCS) alignment strategy.
pub fn compare_histories(
    run_id_a: &str,
    events_a: &[Event],
    run_id_b: &str,
    events_b: &[Event],
) -> DiffReport {
    let n = events_a.len();
    let m = events_b.len();

    // 1. Build LCS Dynamic Programming table based on semantic equality
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in 1..=n {
        for j in 1..=m {
            if events_semantically_equal(&events_a[i - 1], &events_b[j - 1]) {
                dp[i][j] = dp[i - 1][j - 1] + 1;
            } else {
                dp[i][j] = dp[i - 1][j].max(dp[i][j - 1]);
            }
        }
    }

    // 2. Backtrack through DP table to generate raw alignment
    let mut raw_items = Vec::new();
    let mut i = n;
    let mut j = m;

    while i > 0 || j > 0 {
        if i > 0 && j > 0 && events_semantically_equal(&events_a[i - 1], &events_b[j - 1]) {
            raw_items.push(DiffItem {
                step_a: Some(i),
                step_b: Some(j),
                kind: DiffKind::Identical,
                event_a: Some(events_a[i - 1].clone()),
                event_b: Some(events_b[j - 1].clone()),
            });
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
            raw_items.push(DiffItem {
                step_a: None,
                step_b: Some(j),
                kind: DiffKind::Added,
                event_a: None,
                event_b: Some(events_b[j - 1].clone()),
            });
            j -= 1;
        } else if i > 0 {
            raw_items.push(DiffItem {
                step_a: Some(i),
                step_b: None,
                kind: DiffKind::Removed,
                event_a: Some(events_a[i - 1].clone()),
                event_b: None,
            });
            i -= 1;
        }
    }

    raw_items.reverse();

    // 3. Post-process: in contiguous blocks of (Removals, Additions),
    // pair items sharing the same EventType into Modified.
    let mut items: Vec<DiffItem> = Vec::new();
    let mut idx = 0;
    while idx < raw_items.len() {
        if raw_items[idx].kind == DiffKind::Removed {
            let mut rem_block = Vec::new();
            while idx < raw_items.len() && raw_items[idx].kind == DiffKind::Removed {
                rem_block.push(raw_items[idx].clone());
                idx += 1;
            }
            let mut add_block = Vec::new();
            while idx < raw_items.len() && raw_items[idx].kind == DiffKind::Added {
                add_block.push(raw_items[idx].clone());
                idx += 1;
            }

            let mut matched_add = vec![false; add_block.len()];
            for rem in rem_block {
                let ea = rem.event_a.as_ref().unwrap();
                let mut paired = false;
                for (j, add) in add_block.iter().enumerate() {
                    if !matched_add[j] {
                        let eb = add.event_b.as_ref().unwrap();
                        if ea.event_type == eb.event_type {
                            matched_add[j] = true;
                            let field_diffs = diff_payloads(&ea.payload, &eb.payload);
                            items.push(DiffItem {
                                step_a: rem.step_a,
                                step_b: add.step_b,
                                kind: DiffKind::Modified {
                                    field_differences: field_diffs,
                                },
                                event_a: Some(ea.clone()),
                                event_b: Some(eb.clone()),
                            });
                            paired = true;
                            break;
                        }
                    }
                }
                if !paired {
                    items.push(rem);
                }
            }
            for (j, add) in add_block.into_iter().enumerate() {
                if !matched_add[j] {
                    items.push(add);
                }
            }
        } else {
            items.push(raw_items[idx].clone());
            idx += 1;
        }
    }


    // 4. Calculate summary metrics
    let mut identical_count = 0;
    let mut modified_count = 0;
    let mut added_count = 0;
    let mut removed_count = 0;

    for item in &items {
        match item.kind {
            DiffKind::Identical => identical_count += 1,
            DiffKind::Modified { .. } => modified_count += 1,
            DiffKind::Added => added_count += 1,
            DiffKind::Removed => removed_count += 1,
        }
    }

    let first_divergence = find_first_divergence(events_a, events_b);
    let is_identical = first_divergence.is_none()
        && modified_count == 0
        && added_count == 0
        && removed_count == 0;

    DiffReport {
        run_id_a: run_id_a.to_string(),
        run_id_b: run_id_b.to_string(),
        total_events_a: n,
        total_events_b: m,
        is_identical,
        first_divergence,
        identical_count,
        modified_count,
        added_count,
        removed_count,
        items,
    }
}

/// High-level function that loads two runs from a directory and compares them.
pub fn compare_runs(
    runs_dir: impl AsRef<Path>,
    run_id_a: &str,
    run_id_b: &str,
) -> Result<DiffReport, String> {
    let dir = runs_dir.as_ref();

    let path_a = Run::find_path(dir, run_id_a)
        .ok_or_else(|| format!("Run A '{run_id_a}' not found in '{}'", dir.display()))?;
    let path_b = Run::find_path(dir, run_id_b)
        .ok_or_else(|| format!("Run B '{run_id_b}' not found in '{}'", dir.display()))?;

    let events_a = JsonlStorage::read_events(&path_a)
        .map_err(|e| format!("Failed to read Run A events: {e}"))?;
    let events_b = JsonlStorage::read_events(&path_b)
        .map_err(|e| format!("Failed to read Run B events: {e}"))?;

    Ok(compare_histories(run_id_a, &events_a, run_id_b, &events_b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_event(run_id: &str, event_type: EventType, payload: serde_json::Value) -> Event {
        Event::new(run_id, event_type, payload)
    }

    #[test]
    fn test_identical_runs() {
        // Runs have different generated IDs and timestamps, but identical semantics
        let events_a = vec![
            make_event("run-1", EventType::RunStarted, json!({"agent": "a1"})),
            make_event(
                "run-1",
                EventType::ToolStarted,
                json!({"tool": "calc", "args": "1+1"}),
            ),
            make_event(
                "run-1",
                EventType::ToolCompleted,
                json!({"output": 2}),
            ),
            make_event("run-1", EventType::RunCompleted, json!({"status": "ok"})),
        ];

        let events_b = vec![
            make_event("run-2", EventType::RunStarted, json!({"agent": "a1"})),
            make_event(
                "run-2",
                EventType::ToolStarted,
                json!({"tool": "calc", "args": "1+1"}),
            ),
            make_event(
                "run-2",
                EventType::ToolCompleted,
                json!({"output": 2}),
            ),
            make_event("run-2", EventType::RunCompleted, json!({"status": "ok"})),
        ];

        let report = compare_histories("run-1", &events_a, "run-2", &events_b);
        assert!(report.is_identical);
        assert_eq!(report.first_divergence, None);
        assert_eq!(report.identical_count, 4);
        assert_eq!(report.modified_count, 0);
        assert_eq!(report.added_count, 0);
        assert_eq!(report.removed_count, 0);
    }

    #[test]
    fn test_different_event_payloads() {
        let events_a = vec![
            make_event("run-1", EventType::RunStarted, json!({})),
            make_event(
                "run-1",
                EventType::ToolStarted,
                json!({"tool": "read_file", "path": "old_config.json"}),
            ),
            make_event("run-1", EventType::RunCompleted, json!({})),
        ];

        let events_b = vec![
            make_event("run-2", EventType::RunStarted, json!({})),
            make_event(
                "run-2",
                EventType::ToolStarted,
                json!({"tool": "read_file", "path": "new_config.json"}),
            ),
            make_event("run-2", EventType::RunCompleted, json!({})),
        ];

        let report = compare_histories("run-1", &events_a, "run-2", &events_b);
        assert!(!report.is_identical);
        assert_eq!(report.modified_count, 1);
        assert_eq!(report.identical_count, 2);

        let div = report.first_divergence.unwrap();
        assert_eq!(div.step, 2);
        assert!(div.reason.contains("path"));
        assert!(div.reason.contains("old_config.json"));
        assert!(div.reason.contains("new_config.json"));
    }

    #[test]
    fn test_missing_events() {
        // Run A has 4 events; Run B is missing event #2 and #3
        let events_a = vec![
            make_event("run-1", EventType::RunStarted, json!({})),
            make_event("run-1", EventType::ToolStarted, json!({"tool": "ping"})),
            make_event("run-1", EventType::ToolCompleted, json!({"pong": true})),
            make_event("run-1", EventType::RunCompleted, json!({})),
        ];

        let events_b = vec![
            make_event("run-2", EventType::RunStarted, json!({})),
            make_event("run-2", EventType::RunCompleted, json!({})),
        ];

        let report = compare_histories("run-1", &events_a, "run-2", &events_b);
        assert!(!report.is_identical);
        assert_eq!(report.removed_count, 2);
        assert_eq!(report.identical_count, 2);

        let div = report.first_divergence.unwrap();
        assert_eq!(div.step, 2);
    }

    #[test]
    fn test_extra_events() {
        // Run A has 2 events; Run B has 2 extra tool events inserted
        let events_a = vec![
            make_event("run-1", EventType::RunStarted, json!({})),
            make_event("run-1", EventType::RunCompleted, json!({})),
        ];

        let events_b = vec![
            make_event("run-2", EventType::RunStarted, json!({})),
            make_event("run-2", EventType::ToolStarted, json!({"tool": "extra"})),
            make_event("run-2", EventType::ToolCompleted, json!({"res": 1})),
            make_event("run-2", EventType::RunCompleted, json!({})),
        ];

        let report = compare_histories("run-1", &events_a, "run-2", &events_b);
        assert!(!report.is_identical);
        assert_eq!(report.added_count, 2);
        assert_eq!(report.identical_count, 2);

        let div = report.first_divergence.unwrap();
        assert_eq!(div.step, 2);
    }

    #[test]
    fn test_different_event_ordering() {
        // Run A calls Tool A then Tool B; Run B calls Tool B then Tool A
        let events_a = vec![
            make_event("run-1", EventType::RunStarted, json!({})),
            make_event("run-1", EventType::ToolStarted, json!({"tool": "A"})),
            make_event("run-1", EventType::ToolStarted, json!({"tool": "B"})),
            make_event("run-1", EventType::RunCompleted, json!({})),
        ];

        let events_b = vec![
            make_event("run-2", EventType::RunStarted, json!({})),
            make_event("run-2", EventType::ToolStarted, json!({"tool": "B"})),
            make_event("run-2", EventType::ToolStarted, json!({"tool": "A"})),
            make_event("run-2", EventType::RunCompleted, json!({})),
        ];

        let report = compare_histories("run-1", &events_a, "run-2", &events_b);
        assert!(!report.is_identical);

        let div = report.first_divergence.unwrap();
        assert_eq!(div.step, 2); // Diverges right at step 2
    }
}
