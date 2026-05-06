//! # 策略层
//!
//! `core` 保留 durable event 需要序列化的 prompt layer 枚举以及策略审批类型。

mod approval;
mod engine;

pub use approval::{ApprovalDefault, ApprovalRequest};
pub use engine::SystemPromptLayer;
