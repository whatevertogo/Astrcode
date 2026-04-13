use crate::ApplicationError;

/// 执行控制输入。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecutionControl {
    pub token_budget: Option<u64>,
    pub max_steps: Option<u32>,
    pub manual_compact: Option<bool>,
}

impl ExecutionControl {
    pub fn validate(&self) -> Result<(), ApplicationError> {
        if matches!(self.token_budget, Some(0)) {
            return Err(ApplicationError::InvalidArgument(
                "field 'tokenBudget' must be greater than 0".to_string(),
            ));
        }
        if matches!(self.max_steps, Some(0)) {
            return Err(ApplicationError::InvalidArgument(
                "field 'maxSteps' must be greater than 0".to_string(),
            ));
        }
        Ok(())
    }
}
