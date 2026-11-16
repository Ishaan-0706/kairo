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

    let replay = crate::replay::ReplayEngine::from_file(&path)
        .map_err(|e| format!("Failed to load replay engine: {e}"))?;

    println!("Starting replay of run '{}'...", replay.run_id());
    println!("  Log Path:              {}", path.display());
    println!("  Total Recorded Events: {}", replay.total_events());

    let report = replay.finish();
    if report.is_success() {
        println!("\nReplay Succeeded!");
        println!(
            "  Validated {}/{} events without external side-effects.",
            report.total_recorded_events, report.total_recorded_events
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


