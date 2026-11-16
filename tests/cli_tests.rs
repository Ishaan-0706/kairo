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

#[test]
fn test_cli_checkpoint_branch_and_lineage_commands() {
    let dir = temp_cli_dir("cli_cp_branch");
    let file_root = dir.join("run-cli-root.jsonl");

    // 1. Create a root run with 3 events
    let mut root = Run::start_with_id("run-cli-root", &file_root, serde_json::json!({"agent": "main"})).unwrap();
    root.record(kairo::event::EventType::ToolStarted, serde_json::json!({"cmd": "init"})).unwrap();
    root.complete(serde_json::json!({"res": "ok"})).unwrap();

    // 2. Create checkpoint via CLI at step 2
    let cp_cli = Cli {
        dir: dir.clone(),
        command: Commands::Checkpoint {
            command: kairo::cli::CheckpointCommands::Create {
                run_id: "run-cli-root".to_string(),
                step: 2,
                name: "after-init".to_string(),
            },
        },
    };
    assert!(run_cli(cp_cli).is_ok());

    // 3. List checkpoints
    let list_cli = Cli {
        dir: dir.clone(),
        command: Commands::Checkpoint {
            command: kairo::cli::CheckpointCommands::List { run_id: None },
        },
    };
    assert!(run_cli(list_cli).is_ok());

    let checkpoints = kairo::checkpoint::CheckpointManager::list_checkpoints(&dir, None).unwrap();
    assert_eq!(checkpoints.len(), 1);
    let cp_id = checkpoints[0].checkpoint_id.clone();

    // 4. Branch from checkpoint via CLI
    let branch_cli = Cli {
        dir: dir.clone(),
        command: Commands::Branch {
            checkpoint_id: cp_id,
            id: Some("run-cli-branch".to_string()),
            note: "experiment 1".to_string(),
        },
    };
    assert!(run_cli(branch_cli).is_ok());

    // 5. Inspect lineage via CLI
    let lineage_cli = Cli {
        dir: dir.clone(),
        command: Commands::Lineage {
            run_id: "run-cli-branch".to_string(),
        },
    };
    assert!(run_cli(lineage_cli).is_ok());

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn test_cli_serve_argument_parsing() {
    use clap::Parser;
    let cli = Cli::try_parse_from([
        "kairo",
        "--dir",
        "custom/runs",
        "serve",
        "--host",
        "0.0.0.0",
        "--port",
        "8080",
    ])
    .unwrap();
    assert_eq!(cli.dir, PathBuf::from("custom/runs"));
    match cli.command {
        Commands::Serve { host, port } => {
            assert_eq!(host, "0.0.0.0");
            assert_eq!(port, 8080);
        }
        _ => panic!("Expected Commands::Serve"),
    }
}

#[test]
fn test_cli_eval_commands() {
    use kairo::cli::EvalCommands;
    let dir = temp_cli_dir("eval_cli");
    let runs_dir = dir.join("runs");
    fs::create_dir_all(&runs_dir).unwrap();

    // 1. Create baseline run (output: 42)
    let base_file = runs_dir.join("run-eval-base.jsonl");
    let mut base_run = Run::start_with_id("run-eval-base", &base_file, json!({"agent": "calc-agent", "metadata": {"goal": "compute 42"}})).unwrap();
    base_run.record(EventType::ToolStarted, json!({"tool": "calculator"})).unwrap();
    base_run.complete(json!({"result": 42})).unwrap();

    // 2. Create candidate run (output: 99 - regressed)
    let cand_file = runs_dir.join("run-eval-cand.jsonl");
    let mut cand_run = Run::start_with_id("run-eval-cand", &cand_file, json!({"agent": "calc-agent"})).unwrap();
    cand_run.record(EventType::ToolStarted, json!({"tool": "calculator"})).unwrap();
    cand_run.complete(json!({"result": 99})).unwrap();

    // 3. CLI eval create
    let create_cli = Cli {
        dir: runs_dir.clone(),
        command: Commands::Eval {
            command: EvalCommands::Create {
                run_id: "run-eval-base".to_string(),
                id: Some("case-calc".to_string()),
                name: Some("Calculator Goal".to_string()),
            },
        },
    };
    assert!(run_cli(create_cli).is_ok());

    // 4. CLI eval list
    let list_cli = Cli {
        dir: runs_dir.clone(),
        command: Commands::Eval {
            command: EvalCommands::List,
        },
    };
    assert!(run_cli(list_cli).is_ok());

    // 5. CLI eval check on baseline (passes)
    let check_base_cli = Cli {
        dir: runs_dir.clone(),
        command: Commands::Eval {
            command: EvalCommands::Check {
                case_id: "case-calc".to_string(),
                run_id: "run-eval-base".to_string(),
            },
        },
    };
    assert!(run_cli(check_base_cli).is_ok());

    // 6. CLI eval check on candidate (executes and handles mismatch)
    let check_cand_cli = Cli {
        dir: runs_dir.clone(),
        command: Commands::Eval {
            command: EvalCommands::Check {
                case_id: "case-calc".to_string(),
                run_id: "run-eval-cand".to_string(),
            },
        },
    };
    assert!(run_cli(check_cand_cli).is_ok());

    // 7. CLI eval regress (compares baseline and candidate, finds regression)
    let regress_cli = Cli {
        dir: runs_dir.clone(),
        command: Commands::Eval {
            command: EvalCommands::Regress {
                baseline: "run-eval-base".to_string(),
                candidate: "run-eval-cand".to_string(),
                case_id: "case-calc".to_string(),
            },
        },
    };
    assert!(run_cli(regress_cli).is_ok());

    let _ = fs::remove_dir_all(dir);
}



