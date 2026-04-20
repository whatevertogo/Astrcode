pub mod cascade_failure;
pub mod compact_loss;
pub mod empty_turn;
pub mod subrun_budget;
pub mod tool_loop;

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::trace::{SessionTrace, StorageSeqRange, TurnTrace};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum FailureSeverity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FailureInstance {
    pub pattern_name: String,
    pub severity: FailureSeverity,
    pub confidence: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_seq_range: Option<StorageSeqRange>,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TurnDiagnosis {
    pub turn_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failures: Vec<FailureInstance>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosisSummary {
    pub total_failures: usize,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pattern_counts: BTreeMap<String, usize>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub severity_counts: BTreeMap<FailureSeverity, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosisReport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turns: Vec<TurnDiagnosis>,
    pub summary: DiagnosisSummary,
}

pub trait FailurePatternDetector: Send + Sync {
    fn name(&self) -> &'static str;
    fn severity(&self) -> FailureSeverity;
    fn detect(&self, trace: &TurnTrace) -> Vec<FailureInstance>;
}

#[derive(Default)]
pub struct DiagnosisEngine {
    detectors: Vec<Box<dyn FailurePatternDetector>>,
}

impl DiagnosisEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<D>(&mut self, detector: D)
    where
        D: FailurePatternDetector + 'static,
    {
        self.detectors.push(Box::new(detector));
    }

    pub fn diagnose_turn(&self, trace: &TurnTrace) -> TurnDiagnosis {
        let failures = self
            .detectors
            .iter()
            .flat_map(|detector| detector.detect(trace))
            .collect();
        TurnDiagnosis {
            turn_id: trace.turn_id.clone(),
            failures,
        }
    }

    pub fn diagnose_session(&self, trace: &SessionTrace) -> DiagnosisReport {
        let turns: Vec<TurnDiagnosis> = trace
            .turns
            .iter()
            .map(|turn| self.diagnose_turn(turn))
            .filter(|turn| !turn.failures.is_empty())
            .collect();

        let mut summary = DiagnosisSummary::default();
        for failure in turns.iter().flat_map(|turn| turn.failures.iter()) {
            summary.total_failures += 1;
            *summary
                .pattern_counts
                .entry(failure.pattern_name.clone())
                .or_default() += 1;
            *summary.severity_counts.entry(failure.severity).or_default() += 1;
        }

        DiagnosisReport {
            session_id: trace.session_id.clone(),
            turns,
            summary,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{DiagnosisEngine, FailureInstance, FailurePatternDetector, FailureSeverity};
    use crate::trace::TurnTrace;

    struct MockDetector;

    impl FailurePatternDetector for MockDetector {
        fn name(&self) -> &'static str {
            "mock"
        }

        fn severity(&self) -> FailureSeverity {
            FailureSeverity::Low
        }

        fn detect(&self, trace: &TurnTrace) -> Vec<FailureInstance> {
            vec![FailureInstance {
                pattern_name: self.name().to_string(),
                severity: self.severity(),
                confidence: 0.9,
                storage_seq_range: trace.storage_seq_range.clone(),
                description: "mock failure".to_string(),
                context: Some(json!({"turnId": trace.turn_id})),
            }]
        }
    }

    #[test]
    fn diagnosis_engine_registers_and_runs_all_detectors() {
        let mut engine = DiagnosisEngine::new();
        engine.register(MockDetector);

        let report = engine.diagnose_session(&crate::trace::SessionTrace {
            session_id: Some("session-1".to_string()),
            working_dir: None,
            started_at: None,
            parent_session_id: None,
            parent_storage_seq: None,
            turns: vec![TurnTrace {
                turn_id: "turn-1".to_string(),
                user_input: None,
                assistant_output: None,
                assistant_reasoning: None,
                thinking_deltas: Vec::new(),
                tool_calls: Vec::new(),
                prompt_metrics: Vec::new(),
                compactions: Vec::new(),
                sub_runs: Vec::new(),
                collaboration_facts: Vec::new(),
                errors: Vec::new(),
                timeline: Vec::new(),
                agent_lineage: Vec::new(),
                storage_seq_range: None,
                completed_at: None,
                completion_reason: None,
                incomplete: false,
            }],
            agent_lineage: Vec::new(),
        });

        assert_eq!(report.turns.len(), 1);
        assert_eq!(report.summary.total_failures, 1);
        assert_eq!(report.summary.pattern_counts.get("mock"), Some(&1));
    }
}
