use super::{FailureInstance, FailurePatternDetector, FailureSeverity};
use crate::trace::TurnTrace;

#[derive(Default)]
pub struct SubRunBudgetDetector;

impl FailurePatternDetector for SubRunBudgetDetector {
    fn name(&self) -> &'static str {
        "subrun_budget"
    }

    fn severity(&self) -> FailureSeverity {
        FailureSeverity::Medium
    }

    fn detect(&self, _trace: &TurnTrace) -> Vec<FailureInstance> {
        Vec::new()
    }
}
