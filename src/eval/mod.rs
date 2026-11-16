pub mod case;
pub mod metrics;
pub mod regression;

pub use case::{EvalCase, ExpectedOutcome, OutputMatcher};
pub use metrics::{evaluate_run, EvalReport, EvalResult, TokenUsage};
pub use regression::{ImprovementItem, RegressionItem, RegressionReport};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, EventType};
    use serde_json::json;

    fn make_mock_run(
        run_id: &str,
        tool_called: bool,
        output_val: &str,
        usage: Option<(u64, u64)>,
    ) -> Vec<Event> {
        let mut events = vec![Event::new(
            run_id,
            EventType::RunStarted,
            json!({"metadata": {"goal": "Calculate 2 + 2"}}),
        )];

        if tool_called {
            events.push(Event::new(
                run_id,
                EventType::Custom("tool.call".to_string()),
                json!({"tool": "calculator", "arguments": {"expr": "2 + 2"}}),
            ));
            events.push(Event::new(
                run_id,
                EventType::ToolCompleted,
                json!({"tool": "calculator", "output": 4}),
            ));
        }

        let mut comp_payload = json!({
            "result": output_val,
            "status": "success"
        });

        if let Some((prompt_tok, comp_tok)) = usage {
            comp_payload["usage"] = json!({
                "prompt_tokens": prompt_tok,
                "completion_tokens": comp_tok,
                "total_tokens": prompt_tok + comp_tok,
            });
        }

        events.push(Event::new(run_id, EventType::RunCompleted, comp_payload));
        events
    }

    #[test]
    fn test_eval_case_creation_from_recorded_run() {
        let events = make_mock_run("run-src-1", true, "4", None);
        let case = EvalCase::from_run("run-src-1", &events).expect("Failed to create eval case");

        assert_eq!(case.id, "eval-run-src-1");
        assert_eq!(case.source_run_id, Some("run-src-1".to_string()));
        assert_eq!(case.input, json!("Calculate 2 + 2"));
        assert_eq!(case.expected.required_tools, vec!["calculator"]);
        assert_eq!(case.expected.output_matcher, OutputMatcher::Exact(json!("4")));
    }

    #[test]
    fn test_successful_evaluation() {
        let events = make_mock_run("run-cand-1", true, "4", None);
        let case = EvalCase::new(
            "eval-math-4",
            "Test 2 + 2",
            json!("2 + 2"),
            ExpectedOutcome {
                status: "completed".to_string(),
                output_matcher: OutputMatcher::Exact(json!("4")),
                required_tools: vec!["calculator".to_string()],
                max_duration_ms: None,
                max_tokens: None,
            },
        );

        let result = evaluate_run(&case, "run-cand-1", &events);
        assert!(result.passed);
        assert_eq!(result.actual_output, json!("4"));
        assert!(result.failures.is_empty());
    }

    #[test]
    fn test_output_mismatch_failure() {
        // Mock agent outputs wrong result "5"
        let events = make_mock_run("run-cand-wrong", true, "5", None);
        let case = EvalCase::new(
            "eval-math-4",
            "Test 2 + 2",
            json!("2 + 2"),
            ExpectedOutcome {
                status: "completed".to_string(),
                output_matcher: OutputMatcher::Exact(json!("4")),
                required_tools: vec!["calculator".to_string()],
                max_duration_ms: None,
                max_tokens: None,
            },
        );

        let result = evaluate_run(&case, "run-cand-wrong", &events);
        assert!(!result.passed);
        assert!(result.failures[0].contains("Output mismatch"));
    }

    #[test]
    fn test_missing_required_tool_failure() {
        // Agent did not invoke calculator
        let events = make_mock_run("run-cand-no-tool", false, "4", None);
        let case = EvalCase::new(
            "eval-math-4",
            "Test 2 + 2",
            json!("2 + 2"),
            ExpectedOutcome {
                status: "completed".to_string(),
                output_matcher: OutputMatcher::Exact(json!("4")),
                required_tools: vec!["calculator".to_string()],
                max_duration_ms: None,
                max_tokens: None,
            },
        );

        let result = evaluate_run(&case, "run-cand-no-tool", &events);
        assert!(!result.passed);
        assert!(result.failures[0].contains("Missing required tool call: 'calculator'"));
    }

    #[test]
    fn test_token_usage_only_recorded_when_supplied() {
        // Run without provider token usage -> None
        let events_no_tok = make_mock_run("run-no-tok", true, "4", None);
        let usage_none = TokenUsage::extract_from_events(&events_no_tok);
        assert_eq!(usage_none, None);

        // Run with provider token usage -> Some
        let events_with_tok = make_mock_run("run-with-tok", true, "4", Some((120, 35)));
        let usage_some = TokenUsage::extract_from_events(&events_with_tok).unwrap();
        assert_eq!(usage_some.prompt_tokens, Some(120));
        assert_eq!(usage_some.completion_tokens, Some(35));
        assert_eq!(usage_some.total_tokens, Some(155));
    }

    #[test]
    fn test_regression_comparison_detection() {
        let case = EvalCase::new(
            "case-add-1",
            "Addition Test",
            json!("2 + 2"),
            ExpectedOutcome {
                status: "completed".to_string(),
                output_matcher: OutputMatcher::Exact(json!("4")),
                required_tools: vec!["calculator".to_string()],
                max_duration_ms: None,
                max_tokens: None,
            },
        );

        // 1. Baseline: Passed
        let base_events = make_mock_run("run-v1", true, "4", Some((10, 5)));
        let base_res = evaluate_run(&case, "run-v1", &base_events);
        assert!(base_res.passed);
        let base_report = EvalReport::new("report-v1", vec![base_res]);

        // 2. Candidate: Failed (regressed to output 5)
        let cand_events = make_mock_run("run-v2", true, "5", Some((15, 8)));
        let cand_res = evaluate_run(&case, "run-v2", &cand_events);
        assert!(!cand_res.passed);
        let cand_report = EvalReport::new("report-v2", vec![cand_res]);

        // 3. Regression comparison
        let reg_report = RegressionReport::compare(&base_report, &cand_report);
        assert_eq!(reg_report.total_regressions, 1);
        assert_eq!(reg_report.regressions[0].case_id, "case-add-1");
        assert_eq!(reg_report.token_delta, Some(8)); // 23 - 15 = 8
        assert!(reg_report.format_cli().contains("REGRESSIONS DETECTED"));
    }
}
