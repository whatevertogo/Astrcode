//! 共享执行控制输入。
//!
//! 为什么放在 core：
//! `ExecutionControl` 已经被 application、server、client 和 protocol 共同消费，
//! 它描述的是稳定执行语义，而不是某个具体 HTTP route 的 request 壳。

use crate::error::AstrError;

/// 执行控制输入。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionControl {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_compact: Option<bool>,
}

impl ExecutionControl {
    pub fn validate(&self) -> std::result::Result<(), AstrError> {
        Ok(())
    }
}
