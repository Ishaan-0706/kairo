//! End-to-end integration test validating a realistic resilient agent execution,
//! failure recovery, event recording durability, deterministic replay, and evaluation.

use kairo::eval::{evaluate_run, EvalCase, ExpectedOutcome, OutputMatcher};
use kairo::event::EventType;
use kairo::recorder::{Run, RunStatus};
use kairo::replay::ReplayEngine;
use kairo::storage::JsonlStorage;
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

fn temp_e2e_dir(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let unique = Uuid::new_v4().to_string();
    path.push(format!("kairo_e2e_{name}_{unique}"));
    fs::create_dir_all(&path).expect("Failed to create temporary directory");
    path
}

/// Simulated external service environment for realistic agent testing.
struct MockDeploymentEnvironment {
    cluster_online: bool,
    replica_healthy: bool,
}

impl MockDeploymentEnvironment {
    fn new() -> Self {
        Self {
            cluster_online: true,
            replica_healthy: false, // Initially unhealthy to trigger tool failure
        }
    }

    fn check_cluster(&self) -> serde_json::Value {
        json!({
            "status": if self.cluster_online { "online" } else { "offline" },
            "region": "us-east-1",
            "nodes": 3
        })
    }

    fn deploy_service(&self, version: &str) -> Result<serde_json::Value, String> {
        if !self.replica_healthy {
            Err("Database replica #2 unreachable; connection pool exhausted".to_string())
        } else {
            Ok(json!({
                "deployment_id": "dep-9941",
                "version": version,
                "status": "live",
                "healthy_endpoints": 3
            }))
        }
    }

    fn heal_replica(&mut self) -> serde_json::Value {
        self.replica_healthy = true;
        json!({"action": "restart_replica", "replica": 2, "status": "synced"})
    }
}

#[test]
fn test_end_to_end_resilient_agent_workflow() {
    let base_dir = temp_e2e_dir("resilient_agent");
    let run_id = "run-prod-deploy-001";
    let run_path = base_dir.join(format!("{run_id}.jsonl"));

    let mut env = MockDeploymentEnvironment::new();

    // =========================================================================
    // Phase 1: Agent Execution with Tool Failure and Autonomous Recovery
    // =========================================================================
    let task_goal = "Deploy service v1.2.0 to production cluster";
    let mut run = Run::start_with_id(
        run_id,
        &run_path,
        json!({
            "agent": "deploy-agent-v1",
            "metadata": {
                "goal": task_goal,
                "environment": "production"
            }
        }),
    )
    .expect("Failed to start run");

    assert_eq!(run.status(), RunStatus::Running);

    // 1. Initial LLM plan
    run.record(
        EventType::LlmRequest,
        json!({"prompt": format!("Plan and execute: {task_goal}")}),
    )
    .unwrap();

    run.record(
        EventType::LlmResponse,
        json!({"response": "Step 1: Check cluster health before deploying."}),
    )
    .unwrap();

    // 2. Tool 1: cluster_check (Succeeds)
    run.record(
        EventType::Custom("tool.call".to_string()),
        json!({"tool": "cluster_check", "arguments": {}}),
    )
    .unwrap();
    let cluster_status = env.check_cluster();
    run.record(
        EventType::ToolCompleted,
        json!({"tool": "cluster_check", "output": cluster_status}),
    )
    .unwrap();

    // 3. Tool 2: deploy_service (Fails initially)
    run.record(
        EventType::Custom("tool.call".to_string()),
        json!({"tool": "deploy_service", "arguments": {"version": "v1.2.0"}}),
    )
    .unwrap();

    let deploy_res = env.deploy_service("v1.2.0");
    assert!(deploy_res.is_err());
    let err_msg = deploy_res.unwrap_err();

    run.record(
        EventType::ToolFailed,
        json!({"tool": "deploy_service", "error": err_msg}),
    )
    .unwrap();

    // 4. LLM analyzes error and decides to heal the database replica
    run.record(
        EventType::LlmResponse,
        json!({"response": "Deployment failed due to replica #2. Executing heal_replica tool."}),
    )
    .unwrap();

    // 5. Tool 3: heal_replica (Recovery action)
    run.record(
        EventType::Custom("tool.call".to_string()),
        json!({"tool": "heal_replica", "arguments": {"replica": 2}}),
    )
    .unwrap();
    let heal_res = env.heal_replica();
    run.record(
        EventType::ToolCompleted,
        json!({"tool": "heal_replica", "output": heal_res}),
    )
    .unwrap();

    // 6. Tool 4: deploy_service retry (Succeeds after healing)
    run.record(
        EventType::Custom("tool.call".to_string()),
        json!({"tool": "deploy_service", "arguments": {"version": "v1.2.0"}}),
    )
    .unwrap();
    let deploy_retry = env.deploy_service("v1.2.0").expect("Deploy retry failed");
    run.record(
        EventType::ToolCompleted,
        json!({"tool": "deploy_service", "output": deploy_retry}),
    )
    .unwrap();

    // 7. Final synthesis & Task Completion
    let final_answer = "Service v1.2.0 successfully deployed to production after healing replica #2.";
    run.record(
        EventType::LlmResponse,
        json!({"response": final_answer}),
    )
    .unwrap();

    let completion_ev = run
        .complete(json!({
            "result": "success",
            "version": "v1.2.0",
            "recovered_from_failure": true,
            "tool_calls_total": 4,
            "usage": {
                "prompt_tokens": 450,
                "completion_tokens": 120,
                "total_tokens": 570
            }
        }))
        .expect("Failed to complete run");

    assert_eq!(completion_ev.run_id, run_id);
    assert_eq!(run.status(), RunStatus::Completed);

    // =========================================================================
    // Phase 2: Verify Durability & Complete Event Sequence in Storage
    // =========================================================================
    let recorded_events = JsonlStorage::read_events(&run_path).expect("Failed to read events");
    assert_eq!(recorded_events.len(), 14);

    assert_eq!(recorded_events[0].event_type, EventType::RunStarted);
    assert_eq!(recorded_events[1].event_type, EventType::LlmRequest);
    assert_eq!(recorded_events[2].event_type, EventType::LlmResponse);
    assert_eq!(recorded_events[3].event_type, EventType::Custom("tool.call".to_string()));
    assert_eq!(recorded_events[4].event_type, EventType::ToolCompleted);
    assert_eq!(recorded_events[5].event_type, EventType::Custom("tool.call".to_string()));
    assert_eq!(recorded_events[6].event_type, EventType::ToolFailed);
    assert_eq!(recorded_events[7].event_type, EventType::LlmResponse);
    assert_eq!(recorded_events[8].event_type, EventType::Custom("tool.call".to_string()));
    assert_eq!(recorded_events[9].event_type, EventType::ToolCompleted);
    assert_eq!(recorded_events[10].event_type, EventType::Custom("tool.call".to_string()));
    assert_eq!(recorded_events[11].event_type, EventType::ToolCompleted);
    assert_eq!(recorded_events[12].event_type, EventType::LlmResponse);
    assert_eq!(recorded_events[13].event_type, EventType::RunCompleted);

    // =========================================================================
    // Phase 3: Deterministic Replay Without External Services
    // =========================================================================
    let mut replay = ReplayEngine::from_file(&run_path).expect("Failed to initialize replay engine");
    assert_eq!(replay.total_events(), 14);

    // Consumer replays run start
    let r_start = replay.verify_step(EventType::RunStarted, None).unwrap();
    assert_eq!(r_start.run_id, run_id);

    // Consumer replays first LLM turn using saved response
    let llm_resp_1 = replay
        .replay_llm(&json!({"prompt": format!("Plan and execute: {task_goal}")}))
        .unwrap();
    assert!(llm_resp_1["response"].as_str().unwrap().contains("Check cluster health"));

    // Step through the remaining events
    while replay.cursor() < replay.total_events() {
        let next_type = replay.peek_current().unwrap().event_type.clone();
        replay.verify_step(next_type, None).unwrap();
    }

    let report = replay.finish();
    assert!(report.is_success());
    assert_eq!(report.matched_events, 14);

    // =========================================================================
    // Phase 4: Derive Evaluation Case & Benchmark Outcome
    // =========================================================================
    let mut eval_case = EvalCase::from_run(run_id, &recorded_events).expect("Failed to create eval case");
    eval_case.expected = ExpectedOutcome {
        status: "completed".to_string(),
        output_matcher: OutputMatcher::Exact(json!("success")),
        required_tools: vec![
            "cluster_check".to_string(),
            "deploy_service".to_string(),
            "heal_replica".to_string(),
        ],
        max_duration_ms: None,
        max_tokens: Some(1000),
    };

    let eval_res = evaluate_run(&eval_case, run_id, &recorded_events);
    assert!(eval_res.passed, "Evaluation failed: {:?}", eval_res.failures);
    assert_eq!(eval_res.actual_output, json!("success"));
    assert_eq!(eval_res.token_usage.unwrap().total_tokens, Some(570));

    let _ = fs::remove_dir_all(base_dir);
}
