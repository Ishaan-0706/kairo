use kairo::cli::{run_cli, Cli, Commands, RunCommands};
use kairo::event::EventType;
use kairo::run::Run;

use serde_json::json;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

fn temp_cli_dir(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let unique = Uuid::new_v4().to_string();
    path.push(format!("kairo_cli_test_{name}_{unique}"));
    path
}

#[test]
fn test_cli_run_start_and_runs() {
    let dir = temp_cli_dir("start_and_list");

    // 1. Initially, listing runs on nonexistent/empty dir returns Ok with no runs
    let list_cli = Cli {
        dir: dir.clone(),
        command: Commands::Runs,
    };
    assert!(run_cli(list_cli).is_ok());

    // 2. Start a run with explicit ID
    let start_cli = Cli {
        dir: dir.clone(),
        command: Commands::Run {
            command: RunCommands::Start {
                id: Some("run-test-alpha".to_string()),
                agent: "test-bot".to_string(),
            },
        },
    };
    assert!(run_cli(start_cli).is_ok());

    // Verify run file exists on disk
    let run_file = dir.join("run-test-alpha.jsonl");
    assert!(run_file.exists());

    // 3. List runs should now find our run
    let runs_list = Run::list(&dir).expect("Failed to list runs");
    assert_eq!(runs_list.len(), 1);
    assert_eq!(runs_list[0].run_id, "run-test-alpha");

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn test_cli_inspect_successful_run() {
    let dir = temp_cli_dir("inspect_success");
    let run_file = dir.join("run-inspect-42.jsonl");

    // Create a run, record events, and complete it
    let mut run = Run::start_with_id("run-inspect-42", &run_file, json!({"agent": "auditor"})).unwrap();
    run.record(EventType::ToolStarted, json!({"tool": "scanner"})).unwrap();
    run.complete(json!({"summary": "no issues found"})).unwrap();

    // Inspect the run via CLI
    let inspect_cli = Cli {
        dir: dir.clone(),
        command: Commands::Inspect {
            run_id: "run-inspect-42".to_string(),
        },
    };
    assert!(run_cli(inspect_cli).is_ok());

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn test_cli_inspect_invalid_run_id_fails() {
    let dir = temp_cli_dir("inspect_invalid");
    fs::create_dir_all(&dir).unwrap();

    let inspect_cli = Cli {
        dir: dir.clone(),
        command: Commands::Inspect {
            run_id: "nonexistent-run-xyz".to_string(),
        },
    };

    let result = run_cli(inspect_cli);
    assert!(result.is_err());
    let err_msg = result.unwrap_err();
    assert!(err_msg.contains("nonexistent-run-xyz"));
    assert!(err_msg.contains("not found"));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn test_cli_missing_data_directory_handled_safely() {
    let dir = PathBuf::from("/tmp/nonexistent_kairo_dir_12345");
    if dir.exists() {
        let _ = fs::remove_dir_all(&dir);
    }

    // Listing runs on a missing dir must succeed cleanly and report 0 runs
    let list_cli = Cli {
        dir: dir.clone(),
        command: Commands::Runs,
    };
    assert!(run_cli(list_cli).is_ok());

    // Inspecting on a missing dir produces a clean error
    let inspect_cli = Cli {
        dir: dir.clone(),
        command: Commands::Inspect {
            run_id: "any-run".to_string(),
        },
    };
    let inspect_res = run_cli(inspect_cli);
    assert!(inspect_res.is_err());
}

#[test]
fn test_cli_replay_command() {
    let dir = temp_cli_dir("cli_replay");
    let run_file = dir.join("run-to-replay.jsonl");

    // Record a small run
    let mut run = Run::start_with_id("run-to-replay", &run_file, serde_json::json!({"agent": "bot"})).unwrap();
    run.complete(serde_json::json!({"status": "done"})).unwrap();

    // Replay it via CLI
    let replay_cli = Cli {
        dir: dir.clone(),
        command: Commands::Replay {
            run_id: "run-to-replay".to_string(),
        },
    };
    assert!(run_cli(replay_cli).is_ok());

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn test_cli_diff_command() {
    let dir = temp_cli_dir("cli_diff");
    let file_a = dir.join("run-diff-a.jsonl");
    let file_b = dir.join("run-diff-b.jsonl");

    let mut ra = Run::start_with_id("run-diff-a", &file_a, serde_json::json!({"v": 1})).unwrap();
    ra.complete(serde_json::json!({"summary": "done A"})).unwrap();

    let mut rb = Run::start_with_id("run-diff-b", &file_b, serde_json::json!({"v": 2})).unwrap();
    rb.complete(serde_json::json!({"summary": "done B"})).unwrap();

    let diff_cli = Cli {
        dir: dir.clone(),
        command: Commands::Diff {
            run_a: "run-diff-a".to_string(),
            run_b: "run-diff-b".to_string(),
        },
    };
    assert!(run_cli(diff_cli).is_ok());

    let _ = fs::remove_dir_all(dir);
}


