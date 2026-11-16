pub mod event;
pub mod run;
pub mod storage;

use event::{Event, EventType};
use run::{Run, RunError};
use serde_json::json;
use storage::JsonlStorage;
use std::fs;

fn main() {
    println!("--- Kairo: Run Management & Recorder Demo ---\n");

    let log_path = "kairo_run_demo.jsonl";
    let _ = fs::remove_file(log_path);

    // 1. Start a new execution run (automatically records run.started)
    println!("1. Starting run...");
    let mut run = Run::start(
        log_path,
        json!({"agent": "code-analyzer", "goal": "audit imports"}),
    )
    .expect("Failed to start run");

    println!("   Run started: ID = {}", run.run_id());
    println!("   Initial Status = {}\n", run.status());

    // 2. Record operational events
    println!("2. Recording agent actions...");
    run.record(
        EventType::ToolStarted,
        json!({"tool": "read_dir", "path": "src/"}),
    )
    .expect("Failed to record tool.started");

    run.record(
        EventType::ToolCompleted,
        json!({"files": ["event.rs", "storage.rs", "run.rs"], "status": "success"}),
    )
    .expect("Failed to record tool.completed");

    // 3. Test submitting an event with mismatched run ID (should be rejected)
    println!("\n3. Testing mismatched run_id protection...");
    let alien_event = Event::new("run-foreign-999", EventType::ToolStarted, json!({}));
    match run.record_event(&alien_event) {
        Err(RunError::MismatchedRunId { expected, found }) => {
            println!("   [PROTECTION TRIGGERED] Rejected alien event: expected '{expected}', found '{found}'");
        }
        Ok(_) => panic!("Should not allow alien event!"),
        Err(e) => eprintln!("Unexpected error: {e}"),
    }

    // 4. Complete the run
    println!("\n4. Completing the run...");
    run.complete(json!({"summary": "Audit finished without errors"}))
        .expect("Failed to complete run");
    println!("   Final Status = {}\n", run.status());

    // 5. Test invalid lifecycle operation (attempting to record after completion)
    println!("5. Testing invalid lifecycle transition...");
    match run.record(EventType::ToolStarted, json!({})) {
        Err(RunError::InvalidStateTransition { current, attempted }) => {
            println!("   [PROTECTION TRIGGERED] Rejected '{attempted}' because run status is '{current}'");
        }
        Ok(_) => panic!("Should not allow recording on completed run!"),
        Err(e) => eprintln!("Unexpected error: {e}"),
    }

    // 6. Inspect the persisted run log from disk
    println!("\n6. Reading persisted events from '{log_path}':");
    let events = JsonlStorage::read_events(log_path).expect("Failed to read log");
    for (i, ev) in events.iter().enumerate() {
        println!(
            "   [{}] {:<15} | {}",
            i + 1,
            format!("{:?}", ev.event_type),
            ev.payload
        );
    }

    // 7. Reconstruct the run state from disk
    let loaded = Run::load(log_path).expect("Failed to load run from disk");
    println!(
        "\n7. Reconstructed run from disk: ID = {}, Status = {}",
        loaded.run_id(),
        loaded.status()
    );
    println!("\nRun management demo finished successfully.");
}



