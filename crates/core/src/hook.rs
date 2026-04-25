//! # Lifecycle Hook 契约
//!
//! 将"可拦截的生命周期点"和"纯事件广播"分开，避免再引入第二条事实来源。
//! Hook 只负责少数明确的执行节点，且输入输出必须是强类型的。

use serde::{Deserialize, Serialize};

/// 新 hooks catalog 的稳定事件键。
///
/// 这些键是跨 owner 共享的最小语义；具体 payload、effect 解释与调度报告
/// 归属 `agent-runtime`、`host-session` 或 `plugin-host`。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HookEventKey {
    Input,
    Context,
    BeforeAgentStart,
    BeforeProviderRequest,
    ToolCall,
    ToolResult,
    TurnStart,
    TurnEnd,
    SessionBeforeCompact,
    ResourcesDiscover,
    ModelSelect,
}
