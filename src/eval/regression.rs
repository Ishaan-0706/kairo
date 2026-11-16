use crate::eval::metrics::EvalReport;
use serde::{Deserialize, Serialize};

/// An identified regression where a case passed in the baseline but failed in the candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegressionItem {
    pub case_id: String,
    pub baseline_run_id: String,
    pub candidate_run_id: String,
    pub baseline_duration_ms: u64,
    pub candidate_duration_ms: u64,
    pub failures: Vec<String>,
}

/// An identified improvement where a case failed in the baseline but passed in the candidate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImprovementItem {
    pub case_id: String,
    pub baseline_run_id: String,
    pub candidate_run_id: String,
}

/// A comprehensive regression analysis comparing a candidate evaluation run against a baseline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegressionReport {
    pub baseline_id: String,
    pub candidate_id: String,
    pub total_regressions: usize,
    pub total_improvements: usize,
    pub baseline_pass_rate: f64,
    pub candidate_pass_rate: f64,
    pub pass_rate_delta: f64,
    pub duration_delta_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_delta: Option<i64>,
    pub regressions: Vec<RegressionItem>,
    pub improvements: Vec<ImprovementItem>,
}

impl RegressionReport {
    /// Compares two evaluation reports to identify regressions and improvements.
    pub fn compare(baseline: &EvalReport, candidate: &EvalReport) -> Self {
        let mut regressions = Vec::new();
        let mut improvements = Vec::new();

        for cand_res in &candidate.results {
            if let Some(base_res) = baseline.results.iter().find(|r| r.case_id == cand_res.case_id) {
                if base_res.passed && !cand_res.passed {
                    regressions.push(RegressionItem {
                        case_id: cand_res.case_id.clone(),
                        baseline_run_id: base_res.run_id.clone(),
                        candidate_run_id: cand_res.run_id.clone(),
                        baseline_duration_ms: base_res.duration_ms,
                        candidate_duration_ms: cand_res.duration_ms,
                        failures: cand_res.failures.clone(),
                    });
                } else if !base_res.passed && cand_res.passed {
                    improvements.push(ImprovementItem {
                        case_id: cand_res.case_id.clone(),
                        baseline_run_id: base_res.run_id.clone(),
                        candidate_run_id: cand_res.run_id.clone(),
                    });
                }
            }
        }

        let pass_rate_delta = candidate.pass_rate - baseline.pass_rate;
        let duration_delta_ms = candidate.total_duration_ms as i64 - baseline.total_duration_ms as i64;

        let token_delta = match (candidate.total_tokens, baseline.total_tokens) {
            (Some(c), Some(b)) => Some(c as i64 - b as i64),
            _ => None,
        };

        Self {
            baseline_id: baseline.report_id.clone(),
            candidate_id: candidate.report_id.clone(),
            total_regressions: regressions.len(),
            total_improvements: improvements.len(),
            baseline_pass_rate: baseline.pass_rate,
            candidate_pass_rate: candidate.pass_rate,
            pass_rate_delta,
            duration_delta_ms,
            token_delta,
            regressions,
            improvements,
        }
    }

    /// Formats a readable CLI diff report.
    pub fn format_cli(&self) -> String {
        let mut out = String::new();
        out.push_str("============================================================\n");
        out.push_str("              KAIRO EVALUATION REGRESSION REPORT            \n");
        out.push_str("============================================================\n");
        out.push_str(&format!("  Baseline Report:   {}\n", self.baseline_id));
        out.push_str(&format!("  Candidate Report:  {}\n", self.candidate_id));
        out.push_str(&format!(
            "  Pass Rate:         {:.1}% -> {:.1}% ({:+.1}%)\n",
            self.baseline_pass_rate, self.candidate_pass_rate, self.pass_rate_delta
        ));
        out.push_str(&format!(
            "  Execution Time:    {:+} ms\n",
            self.duration_delta_ms
        ));

        if let Some(tok_delta) = self.token_delta {
            out.push_str(&format!("  Token Delta:       {tok_delta:+}\n"));
        }

        out.push_str("------------------------------------------------------------\n");

        if self.total_regressions == 0 && self.total_improvements == 0 {
            out.push_str("  [PASS] No regressions detected. Behavior is consistent.\n");
        } else {
            if self.total_regressions > 0 {
                out.push_str(&format!(
                    "  [!] REGRESSIONS DETECTED ({}):\n",
                    self.total_regressions
                ));
                for reg in &self.regressions {
                    out.push_str(&format!("    - Case '{}':\n", reg.case_id));
                    out.push_str(&format!(
                        "        Runs: baseline={} -> candidate={}\n",
                        reg.baseline_run_id, reg.candidate_run_id
                    ));
                    for f in &reg.failures {
                        out.push_str(&format!("        Reason: {f}\n"));
                    }
                }
            }

            if self.total_improvements > 0 {
                out.push_str(&format!(
                    "  [*] IMPROVEMENTS DETECTED ({}):\n",
                    self.total_improvements
                ));
                for imp in &self.improvements {
                    out.push_str(&format!(
                        "    + Case '{}' (fixed in {})\n",
                        imp.case_id, imp.candidate_run_id
                    ));
                }
            }
        }
        out.push_str("============================================================\n");

        out
    }
}
