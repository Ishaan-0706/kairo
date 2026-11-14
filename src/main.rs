pub mod event;

use event::{Event, EventType};
use serde_json::json;

fn main() {
    println!("--- Kairo: Event Data Model Example ---\n");

    // 1. Create a sample Event
    let payload = json!({
        "tool": "read_file",
        "path": "src/main.rs",
        "bytes_read": 128,
        "status": "success"
    });

    let event = Event::new("run-001", EventType::ToolCompleted, payload);

    // 2. Serialize to formatted JSON string
    let json_output = match serde_json::to_string_pretty(&event) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("Error serializing event: {err}");
            return;
        }
    };

    println!("Serialized JSON:\n{json_output}\n");

    // 3. Deserialize back from JSON into an Event struct
    match serde_json::from_str::<Event>(&json_output) {
        Ok(deserialized) => {
            println!("Successfully deserialized back to Event!");
            println!("  Event ID:   {}", deserialized.event_id);
            println!("  Run ID:     {}", deserialized.run_id);
            println!("  Event Type: {:?}", deserialized.event_type);
            println!("  Timestamp:  {}", deserialized.timestamp);
            println!("  Payload:    {}", deserialized.payload);
        }
        Err(err) => {
            eprintln!("Error deserializing event: {err}");
        }
    }
}

