pub mod event;
pub mod storage;

use event::{Event, EventType};
use serde_json::json;
use storage::JsonlStorage;
use std::fs;

fn main() {
    println!("--- Kairo: JSONL Storage Engine Demo ---\n");

    let log_path = "kairo_demo.jsonl";

    // Clean up any old demo file
    let _ = fs::remove_file(log_path);

    // 1. Create and append Event 1
    let event1 = Event::new(
        "run-101",
        EventType::RunStarted,
        json!({"agent": "coding-assistant", "task": "refactor code"}),
    );
    println!("1. Appending Event 1 (run.started)...");
    JsonlStorage::append_event(log_path, &event1).expect("Failed to append event 1");

    // 2. Create and append Event 2
    let event2 = Event::new(
        "run-101",
        EventType::ToolStarted,
        json!({"tool": "read_file", "path": "src/storage.rs"}),
    );
    println!("2. Appending Event 2 (tool.started)...");
    JsonlStorage::append_event(log_path, &event2).expect("Failed to append event 2");

    // 3. Create and append Event 3
    let event3 = Event::new(
        "run-101",
        EventType::ToolCompleted,
        json!({"tool": "read_file", "bytes_read": 512, "status": "success"}),
    );
    println!("3. Appending Event 3 (tool.completed)...");
    JsonlStorage::append_event(log_path, &event3).expect("Failed to append event 3");

    // 4. Read back all events from the JSONL file
    println!("\n4. Reading events back from '{log_path}'...");
    let stored_events = JsonlStorage::read_events(log_path).expect("Failed to read events");

    println!("Successfully read {} events from disk:\n", stored_events.len());
    for (i, ev) in stored_events.iter().enumerate() {
        println!(
            "  [{}] Type: {:<15} | Event ID: {} | Run: {}",
            i + 1,
            format!("{:?}", ev.event_type),
            ev.event_id,
            ev.run_id
        );
        println!("      Payload: {}", ev.payload);
    }

    println!("\nStorage demo finished successfully.");
}


