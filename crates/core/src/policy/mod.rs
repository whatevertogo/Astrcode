//! # 策略引擎
//!
//! 定义了策略引擎的抽象接口，用于控制 Agent 的行为。
//!
//! ## 核心职责
//!
//! - 审批流程：决定能力调用是否需要用户确认
//! - 内容审查：检查/重写 LLM 请求
//! - 模型/工具护栏：统一的审批与请求检查入口

mod engine;

pub use engine::{
    AllowAllPolicyEngine, ApprovalDefault, ApprovalPending, ApprovalRequest, ApprovalResolution,
    CapabilityCall, ModelRequest, PolicyContext, PolicyEngine, PolicyVerdict,
};
