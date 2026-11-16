use crate::event::EventType;
use crate::run::Run;
use crate::storage::JsonlStorage;
use clap::{Parser, Subcommand};
use serde_json::json;
use std::path::PathBuf;

/// Command-line interface definition for Kairo.
#[derive(Parser, Debug)]
#[command(name = "kairo")]
#[command(about = "Git-like execution infrastructure for AI agents", version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Data directory for run logs (default: .kairo/runs)
    #[arg(short, long, global = true, default_value = ".kairo/runs")]
    pub dir: PathBuf,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Manage individual agent runs
    Run {
        #[command(subcommand)]
        command: RunCommands,
    },
    /// List all agent execution runs
    Runs,
    /// Inspect the detailed event log and timeline of a run
    Inspect {
        /// The unique ID of the run to inspect
        run_id: String,
    },
    /// Replay a previously recorded run without contacting external services
    Replay {
        /// The unique ID of the run to replay
        run_id: String,
    },
    /// Compare two execution runs and identify differences
    Diff {
        /// The first run ID (baseline reference)
        run_a: String,
        /// The second run ID (comparison target)
        run_b: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum RunCommands {
    /// Start a new execution run
    Start {
        /// Optional custom run identifier
        #[arg(short, long)]
        id: Option<String>,
        /// Optional agent name or metadata
        #[arg(short, long, default_value = "default-agent")]
        agent: String,
    },
}

/// Executes the CLI command based on parsed arguments.
pub fn run_cli(cli: Cli) -> Result<(), String> {
    match cli.command {
        Commands::Run { command } => match command {
            RunCommands::Start { id, agent } => handle_run_start(cli.dir, id, agent),
        },
        Commands::Runs => handle_runs(cli.dir),
        Commands::Inspect { run_id } => handle_inspect(cli.dir, &run_id),
        Commands::Replay { run_id } => handle_replay(cli.dir, &run_id),
        Commands::Diff { run_a, run_b } => handle_diff(cli.dir, &run_a, &run_b),
    }
}



fn handle_run_start(dir: PathBuf, custom_id: Option<String>, agent: String) -> Result<(), String> {
    let run_id = custom_id.unwrap_or_else(|| format!("run-{}", uuid::Uuid::new_v4()));
    let file_path = dir.join(format!("{run_id}.jsonl"));

    let payload = json!({
        "agent": agent,
        "initiated_by": "kairo-cli",
    });

    let run = Run::start_with_id(&run_id, &file_path, payload)
        .map_err(|e| format!("Failed to start run: {e}"))?;

    println!("Started new run:");
    println!("  Run ID:   {}", run.run_id());
    println!("  Status:   {}", run.status());
    println!("  Storage:  {}", run.storage_path().display());
    Ok(())
}

fn handle_runs(dir: PathBuf) -> Result<(), String> {
    let summaries = Run::list(&dir).map_err(|e| format!("Failed to list runs: {e}"))?;

    if summaries.is_empty() {
        println!("No runs found in '{}'.", dir.display());
        return Ok(());
    }

    println!("Recorded Runs ({}):", summaries.len());
    println!(
        "  {:<38} {:<12} {:<8} {:<22}",
        "RUN ID", "STATUS", "EVENTS", "STARTED AT"
    );
    println!("  {}", "-".repeat(82));

    for s in summaries {
        println!(
            "  {:<38} {:<12} {:<8} {:<22}",
            s.run_id,
            format!("{}", s.status),
            s.event_count,
            s.start_time
        );
    }
    Ok(())
}

fn handle_inspect(dir: PathBuf, run_id: &str) -> Result<(), String> {
    let path = Run::find_path(&dir, run_id)
        .ok_or_else(|| format!("Run '{run_id}' not found in '{}'", dir.display()))?;

    let events = JsonlStorage::read_events(&path)
        .map_err(|e| format!("Failed to read run events: {e}"))?;

    if events.is_empty() {
        return Err(format!("Run file '{}' contains no events", path.display()));
    }

    let first = &events[0];
    let mut status = "running";
    for ev in events.iter().skip(1) {
        match ev.event_type {
            EventType::RunCompleted => status = "completed",
            EventType::RunFailed => status = "failed",
            _ => {}
        }
    }

    println!("Run Inspection: {}", first.run_id);
    println!("  Status:        {status}");
    println!("  Started:       {}", first.timestamp);
    println!("  File:          {}", path.display());
    println!("  Total Events:  {}\n", events.len());

    println!("Event Timeline:");
    for (i, ev) in events.iter().enumerate() {
        let type_name = match &ev.event_type {
            EventType::RunStarted => "run.started",
            EventType::RunCompleted => "run.completed",
            EventType::RunFailed => "run.failed",
            EventType::LlmRequest => "llm.request",
            EventType::LlmResponse => "llm.response",
            EventType::ToolStarted => "tool.started",
            EventType::ToolCompleted => "tool.completed",
            EventType::ToolFailed => "tool.failed",
            EventType::Custom(s) => s.as_str(),
        };
        println!(
            "  [{}] {} | {:<15} | {}",
            i + 1,
            ev.timestamp,
            type_name,
            ev.payload
        );
    }

    Ok(())
}

fn handle_replay(dir: PathBuf, run_id: &str) -> Result<(), String> {
    let path = Run::find_path(&dir, run_id)
        .ok_or_else(|| format!("Run '{run_id}' not found in '{}'", dir.display()))?;

    let mut replay = crate::replay::ReplayEngine::from_file(&path)
        .map_err(|e| format!("Failed to load replay engine: {e}"))?;

    println!("Starting replay of run '{}'...", replay.run_id());
    println!("  Log Path:              {}", path.display());
    println!("  Total Recorded Events: {}\n", replay.total_events());

    // Step through the recorded events in sequence
    let total = replay.total_events();
    while replay.cursor() < total {
        let current_index = replay.cursor();
        let next_event = replay.peek_current().cloned();

        match next_event {
            Some(ev) => match ev.event_type {
                EventType::LlmRequest => {
                    print!("  [{}] Replaying LLM request...", current_index + 1);
                    match replay.replay_llm(&ev.payload) {
                        Ok(resp) => {
                            println!(" (ok)");
                            println!("      -> Saved response: {}", resp);
                        }
                        Err(div) => {
                            println!(" (diverged)");
                            eprintln!("  [DIVERGENCE] {div}");
                            break;
                        }
                    }
                }
                EventType::ToolStarted => {
                    print!("  [{}] Replaying tool call...", current_index + 1);
                    match replay.replay_tool(&ev.payload) {
                        Ok((_type, output)) => {
                            println!(" (ok - side effects skipped)");
                            println!("      -> Saved output: {}", output);
                        }
                        Err(div) => {
                            println!(" (diverged)");
                            eprintln!("  [DIVERGENCE] {div}");
                            break;
                        }
                    }
                }
                _ => {
                    let type_label = format!("{:?}", ev.event_type);
                    print!("  [{}] Step {:<18}", current_index + 1, type_label);
                    match replay.verify_step(ev.event_type, Some(&ev.payload)) {
                        Ok(_) => println!(" (ok)"),
                        Err(div) => {
                            println!(" (diverged)");
                            eprintln!("  [DIVERGENCE] {div}");
                            break;
                        }
                    }
                }
            },
            None => break,
        }
    }

    let report = replay.finish();
    if report.is_success() {
        println!("\nReplay Succeeded!");
        println!(
            "  Validated {}/{} events without external side-effects.",
            report.matched_events, report.total_recorded_events
        );
        println!("  All recorded LLM and tool responses verified for offline replay.");
    } else {
        println!("\nReplay Divergence Detected:");
        for div in &report.divergences {
            println!("  - {div}");
        }
    }
    Ok(())
}

fn handle_diff(dir: PathBuf, run_a: &str, run_b: &str) -> Result<(), String> {
    let report = crate::compare::compare_runs(&dir, run_a, run_b)?;

    println!("Comparing Runs:");
    println!(
        "  Run A (baseline):   {} ({} events)",
        report.run_id_a, report.total_events_a
    );
    println!(
        "  Run B (comparison): {} ({} events)\n",
        report.run_id_b, report.total_events_b
    );

    if report.is_identical {
        println!("Outcome: IDENTICAL");
        println!(
            "  All {} events match semantically in order and payload.",
            report.identical_count
        );
        println!("  (Volatile fields like generated IDs and timestamps differed, but execution is identical).");
        return Ok(());
    }

    println!("Outcome: DIVERGENCE DETECTED");
    if let Some(div) = &report.first_divergence {
        println!("  First Divergence at Step {}:", div.step);
        println!("    Reason: {}", div.reason);
        if let (Some(ea), Some(eb)) = (&div.event_a_type, &div.event_b_type) {
            println!("    Run A:  {ea:?}");
            println!("    Run B:  {eb:?}");
        }
    }

    println!("\nDiff Summary:");
    println!(
        "  Identical: {} | Modified: {} | Added: {} | Removed: {}\n",
        report.identical_count, report.modified_count, report.added_count, report.removed_count
    );

    println!("Event Alignment Timeline:");
    println!(
        "  {:<3} {:<10} {:<10} {:<18} DETAILS",
        "STS", "STEP A", "STEP B", "EVENT TYPE"
    );
    println!("  {}", "-".repeat(78));

    for item in &report.items {
        let step_a_str = item
            .step_a
            .map(|s| s.to_string())
            .unwrap_or_else(|| "-".to_string());
        let step_b_str = item
            .step_b
            .map(|s| s.to_string())
            .unwrap_or_else(|| "-".to_string());

        let (tag, type_name, detail) = match &item.kind {
            crate::compare::DiffKind::Identical => {
                let t = item
                    .event_a
                    .as_ref()
                    .map(|e| format!("{:?}", e.event_type))
                    .unwrap_or_default();
                ("=", t, "".to_string())
            }
            crate::compare::DiffKind::Modified { field_differences } => {
                let t = item
                    .event_a
                    .as_ref()
                    .map(|e| format!("{:?}", e.event_type))
                    .unwrap_or_default();
                ("~", t, format!("MODIFIED [{}]", field_differences.join(", ")))
            }
            crate::compare::DiffKind::Removed => {
                let t = item
                    .event_a
                    .as_ref()
                    .map(|e| format!("{:?}", e.event_type))
                    .unwrap_or_default();
                ("-", t, "REMOVED (only in A)".to_string())
            }
            crate::compare::DiffKind::Added => {
                let t = item
                    .event_b
                    .as_ref()
                    .map(|e| format!("{:?}", e.event_type))
                    .unwrap_or_default();
                ("+", t, "ADDED (only in B)".to_string())
            }
        };

        println!(
            "  [{}] {:<10} {:<10} {:<18} {}",
            tag, step_a_str, step_b_str, type_name, detail
        );
    }

    Ok(())
}




