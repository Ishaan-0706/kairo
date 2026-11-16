use kairo::event::EventType;
use kairo::recorder::Run;
use serde_json::json;
use std::env;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = env::args().collect();
    let is_buggy = args.iter().any(|a| a == "--buggy");
    let run_id = if is_buggy {
        "run-agent-buggy".to_string()
    } else {
        "run-agent-correct".to_string()
    };

    let runs_dir = PathBuf::from(".kairo/runs");
    std::fs::create_dir_all(&runs_dir).expect("Failed to create .kairo/runs");
    let file_path = runs_dir.join(format!("{run_id}.jsonl"));

    println!("Running mock agent (mode: {})...", if is_buggy { "BUGGY" } else { "CORRECT" });

    let mut run = Run::start_with_id(
        &run_id,
        &file_path,
        json!({
            "agent": "mock-math-agent",
            "metadata": {"goal": "Calculate (48 * 12) + (360 / 6)"}
        }),
    )
    .expect("Failed to start run");

    // LLM step
    run.record(
        EventType::LlmRequest,
        json!({"prompt": "Calculate (48 * 12) + (360 / 6)"}),
    )
    .unwrap();

    // Tool step
    run.record(
        EventType::Custom("tool.call".to_string()),
        json!({"tool": "calculator", "arguments": {"a": 48, "b": 12, "op": "mul"}}),
    )
    .unwrap();

    run.record(
        EventType::ToolCompleted,
        json!({"tool": "calculator", "output": 576}),
    )
    .unwrap();

    // Final completion
    if is_buggy {
        // Deliberate regression: agent outputs 42.0 instead of 636.0
        run.complete(json!({
            "result": 42.0,
            "status": "success",
            "usage": {
                "prompt_tokens": 120,
                "completion_tokens": 45,
                "total_tokens": 165
            }
        }))
        .unwrap();
        println!("Recorded buggy run '{run_id}' with incorrect output 42.0");
    } else {
        // Correct output
        run.complete(json!({
            "result": 636.0,
            "status": "success",
            "usage": {
                "prompt_tokens": 110,
                "completion_tokens": 30,
                "total_tokens": 140
            }
        }))
        .unwrap();
        println!("Recorded correct run '{run_id}' with expected output 636.0");
    }
}
