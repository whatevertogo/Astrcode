//! # Agent 基础类型
//!
//! 定义 Agent 控制平面需要复用的稳定 DTO：
//! - `AgentProfile`：Agent 能力画像
//! - `AgentStatus`：Agent 生命周期状态
//! - `SubAgentHandle`：对子 Agent 的轻量句柄
//! - `AgentEventContext`：附着在 turn 事件上的父子关系元数据

use serde::{Deserialize, Serialize};

/// Agent 可见模式。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AgentMode {
    /// 只能作为主 Agent 使用。
    Primary,
    /// 只能作为子 Agent 使用。
    SubAgent,
    /// 主/子 Agent 均可使用。
    All,
}

/// Agent 执行状态。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AgentStatus {
    /// 已注册但尚未开始执行。
    Pending,
    /// 正在执行中。
    Running,
    /// 正常完成。
    Completed,
    /// 被取消。
    Cancelled,
    /// 失败结束。
    Failed,
}

impl AgentStatus {
    /// 判断当前状态是否已经到达终态。
    pub fn is_final(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}

/// Agent 画像定义。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    /// Profile 唯一标识。
    pub id: String,
    /// 人类可读名称。
    pub name: String,
    /// 作用说明，供路由/提示词/UI 复用。
    pub description: String,
    /// 该 profile 允许的使用模式。
    pub mode: AgentMode,
    /// 子 Agent 专用系统提示，可为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// 允许使用的工具集合；为空表示由上层策略决定。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    /// 显式禁止的工具集合。
    ///
    /// 该字段用于保留 Claude 风格 agent 定义里的 denylist 语义，
    /// 即使当前策略层还未完整消费，也不能在加载阶段静默丢失。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disallowed_tools: Vec<String>,
    /// 最大 step 数上限。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_steps: Option<u32>,
    /// token 预算上限。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u64>,
    /// 模型偏好。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_preference: Option<String>,
}

/// 子 Agent 的轻量运行句柄。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentHandle {
    /// 运行时分配的 agent 实例 ID。
    pub agent_id: String,
    /// 子 Agent 所属 session。
    pub session_id: String,
    /// 当前子 Agent 在父子树中的深度。
    ///
    /// 这里使用 1-based 深度，首层子 Agent 为 1，便于直接和配置里的 max depth 对齐。
    pub depth: usize,
    /// 触发该子 Agent 的父 turn。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_turn_id: Option<String>,
    /// 该实例绑定的 profile ID。
    pub agent_profile: String,
    /// 当前状态。
    pub status: AgentStatus,
}

/// turn 级事件的 Agent 元数据。
///
/// 这组字段会附着在 `StorageEvent` / `AgentEvent` 上，用于表达：
/// - 事件属于哪个 Agent 实例
/// - 这个 Agent 是由哪个父 turn 触发的
/// - 该 Agent 使用了哪个 profile
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentEventContext {
    /// 事件所属的 agent 实例 ID。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    /// 父 turn ID。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_turn_id: Option<String>,
    /// 使用的 profile ID。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_profile: Option<String>,
}

impl AgentEventContext {
    /// 构造一个子 Agent 事件上下文。
    pub fn subagent(
        agent_id: impl Into<String>,
        parent_turn_id: impl Into<String>,
        agent_profile: impl Into<String>,
    ) -> Self {
        Self {
            agent_id: Some(agent_id.into()),
            parent_turn_id: Some(parent_turn_id.into()),
            agent_profile: Some(agent_profile.into()),
        }
    }

    /// 判断是否为空上下文。
    pub fn is_empty(&self) -> bool {
        self.agent_id.is_none() && self.parent_turn_id.is_none() && self.agent_profile.is_none()
    }
}
