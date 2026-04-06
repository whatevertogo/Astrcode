//! # Agent 基础类型
//!
//! 定义 Agent / 子会话控制平面需要复用的稳定 DTO。
//! 这里刻意把“Agent 实例”和“受控子会话执行域”拆开，
//! 这样 runtime、存储事件、SSE 和 UI 都能围绕同一套语义建模。

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

/// 统一执行入口的调用来源。
///
/// 显式字段比“根据 parent_turn_id 是否为空推断”更稳定，
/// 因为日志、指标和 UI 都需要可靠地区分根执行与子执行。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum InvocationKind {
    /// 父 turn 下的受控子会话执行。
    SubRun,
    /// 顶层独立执行（例如未来的 `/agents/{id}/execute`）。
    RootExecution,
}

/// 子会话事件写入的存储模式。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SubRunStorageMode {
    /// 与父级共享同一个 session event log。
    SharedSession,
    /// 使用独立 child session；当前仅作为实验特性启用。
    IndependentSession,
}

/// 子执行输出引用。
///
/// 这里只做轻量引用，不在本轮引入重量级 artifact 平台，
/// 避免把“子会话语义”实现膨胀成“产物管理系统”。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRef {
    pub kind: String,
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

/// 子执行结果状态。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubRunOutcome {
    Completed,
    Failed { error: String },
    Aborted,
    TokenExceeded,
}

impl SubRunOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Failed { .. } => "failed",
            Self::Aborted => "aborted",
            Self::TokenExceeded => "token_exceeded",
        }
    }
}

/// 子执行结构化结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubRunResult {
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<String>,
    pub status: SubRunOutcome,
}

/// 调用侧可传入的子会话上下文 override。
///
/// 使用 `Option` 字段而不是硬编码完整配置，原因是调用方通常只覆写极少数字段；
/// 其余维度应继续沿用 runtime 的默认强隔离策略。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubagentContextOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_mode: Option<SubRunStorageMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_system_instructions: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_project_instructions: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_working_dir: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_policy_upper_bound: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_cancel_token: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_compact_summary: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_recent_tail: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_recovery_refs: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_parent_findings: Option<bool>,
}

/// 解析后的子会话 override 快照。
///
/// 该结构会被事件和状态查询复用，便于调试“最终到底继承了什么”。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedSubagentContextOverrides {
    pub storage_mode: SubRunStorageMode,
    pub inherit_system_instructions: bool,
    pub inherit_project_instructions: bool,
    pub inherit_working_dir: bool,
    pub inherit_policy_upper_bound: bool,
    pub inherit_cancel_token: bool,
    pub include_compact_summary: bool,
    pub include_recent_tail: bool,
    pub include_recovery_refs: bool,
    pub include_parent_findings: bool,
}

impl Default for ResolvedSubagentContextOverrides {
    fn default() -> Self {
        Self {
            storage_mode: SubRunStorageMode::SharedSession,
            inherit_system_instructions: true,
            inherit_project_instructions: true,
            inherit_working_dir: true,
            inherit_policy_upper_bound: true,
            inherit_cancel_token: true,
            include_compact_summary: true,
            include_recent_tail: false,
            include_recovery_refs: false,
            include_parent_findings: false,
        }
    }
}

/// 解析后的执行限制快照。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedExecutionLimitsSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_steps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
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

/// 受控子会话的轻量运行句柄。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubRunHandle {
    /// 稳定的子执行域 ID。
    pub sub_run_id: String,
    /// 运行时分配的 agent 实例 ID。
    pub agent_id: String,
    /// 子会话写入所在的 session。
    pub session_id: String,
    /// 若使用独立子会话，这里记录 child session id。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
    /// 当前子 Agent 在父子树中的深度。
    pub depth: usize,
    /// 触发该子会话的父 turn。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_turn_id: Option<String>,
    /// 触发该子会话的父 agent。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<String>,
    /// 该实例绑定的 profile ID。
    pub agent_profile: String,
    /// 当前存储模式。
    pub storage_mode: SubRunStorageMode,
    /// 当前状态。
    pub status: AgentStatus,
}

/// turn 级事件的 Agent 元数据。
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
    /// 受控子会话执行域 ID。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_run_id: Option<String>,
    /// 执行来源。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_kind: Option<InvocationKind>,
    /// 事件写入模式。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_mode: Option<SubRunStorageMode>,
    /// 独立子会话 ID（若存在）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
}

impl AgentEventContext {
    /// 构造一个子会话事件上下文。
    pub fn sub_run(
        agent_id: impl Into<String>,
        parent_turn_id: impl Into<String>,
        agent_profile: impl Into<String>,
        sub_run_id: impl Into<String>,
        storage_mode: SubRunStorageMode,
        child_session_id: Option<String>,
    ) -> Self {
        Self {
            agent_id: Some(agent_id.into()),
            parent_turn_id: Some(parent_turn_id.into()),
            agent_profile: Some(agent_profile.into()),
            sub_run_id: Some(sub_run_id.into()),
            invocation_kind: Some(InvocationKind::SubRun),
            storage_mode: Some(storage_mode),
            child_session_id,
        }
    }

    /// 为根执行构造事件上下文。
    pub fn root_execution(agent_id: impl Into<String>, agent_profile: impl Into<String>) -> Self {
        Self {
            agent_id: Some(agent_id.into()),
            parent_turn_id: None,
            agent_profile: Some(agent_profile.into()),
            sub_run_id: None,
            invocation_kind: Some(InvocationKind::RootExecution),
            storage_mode: None,
            child_session_id: None,
        }
    }

    /// 判断是否为空上下文。
    pub fn is_empty(&self) -> bool {
        self.agent_id.is_none()
            && self.parent_turn_id.is_none()
            && self.agent_profile.is_none()
            && self.sub_run_id.is_none()
            && self.invocation_kind.is_none()
            && self.storage_mode.is_none()
            && self.child_session_id.is_none()
    }
}
