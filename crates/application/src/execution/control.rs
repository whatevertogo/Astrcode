use crate::ApplicationError;

/// 执行控制输入。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecutionControl {
    pub max_steps: Option<u32>,
    pub manual_compact: Option<bool>,
}

impl ExecutionControl {
    pub fn validate(&self) -> Result<(), ApplicationError> {
        if matches!(self.max_steps, Some(0)) {
            return Err(ApplicationError::InvalidArgument(
                "field 'maxSteps' must be greater than 0".to_string(),
            ));
        }
        Ok(())
    }
}
