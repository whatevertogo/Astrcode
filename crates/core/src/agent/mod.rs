//! # Agent 基础类型
//!
//! 定义 Agent / 子会话控制平面需要复用的稳定 DTO。
//! 这里刻意把”Agent 实例”和”受控子会话执行域”拆开，
//! 这样 runtime、存储事件、SSE 和 UI 都能围绕同一套语义建模。
//!
//! 子模块划分：
//! - `lifecycle`：AgentLifecycleStatus + AgentTurnOutcome（四工具模型的状态拆层）
//! - `input queue`：durable input queue 信封、事件载荷、四工具参数和 observe 快照

pub mod executor;
pub mod input_queue;
pub mod lifecycle;

use serde::{Deserialize, Serialize};

use crate::{
    AgentId, DeliveryId, SessionId, SubRunId, TurnId,
    error::{AstrError, Result},
};

fn require_non_empty_trimmed(field: &str, value: impl AsRef<str>) -> Result<()> {
    if value.as_ref().trim().is_empty() {
        return Err(AstrError::Validation(format!("{field} 不能为空")));
    }
    Ok(())
}

fn require_not_whitespace_only(field: &str, value: impl AsRef<str>) -> Result<()> {
    let value = value.as_ref();
    if !value.is_empty() && value.trim().is_empty() {
        return Err(AstrError::Validation(format!("{field} 不能为纯空白")));
    }
    Ok(())
}

/// 归一化一个非空白、无重复的字符串列表，并保留首次出现顺序。
pub fn normalize_non_empty_unique_string_list(
    values: &[String],
    field: &str,
) -> Result<Vec<String>> {
    let mut normalized = Vec::with_capacity(values.len());
    let mut seen = std::collections::BTreeSet::new();

    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(AstrError::Validation(format!("{field} 不能包含空字符串")));
        }
        if !seen.insert(trimmed.to_string()) {
            return Err(AstrError::Validation(format!(
                "{field} 不能包含重复项: {trimmed}"
            )));
        }
        normalized.push(trimmed.to_string());
    }

    Ok(normalized)
}

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

/// Fork 上下文继承模式。
///
/// runtime 会用它裁剪子 agent 继承的父对话 tail。
/// 参考 Codex 的 SpawnAgentForkMode 设计。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ForkMode {
    /// 继承完整对话历史。
    FullHistory,
    /// 只继承最近 N 轮对话。
    LastNTurns(usize),
}

/// `spawn` 的稳定调用参数。
///
/// 该 DTO 下沉到 core，是为了让工具层和执行装配层共享同一份参数语义，
/// 避免 `runtime-execution` 只为了复用字段定义而反向依赖 `runtime-agent-tool`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpawnCapabilityGrant {
    /// 本次 child 允许使用的 tool capability names。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
}

impl SpawnCapabilityGrant {
    pub fn validate(&self) -> Result<()> {
        let normalized = normalize_non_empty_unique_string_list(
            &self.allowed_tools,
            "capabilityGrant.allowedTools",
        )?;
        if normalized.is_empty() {
            return Err(AstrError::Validation(
                "capabilityGrant.allowedTools 不能为空".to_string(),
            ));
        }
        Ok(())
    }

    pub fn normalized_allowed_tools(&self) -> Result<Vec<String>> {
        normalize_non_empty_unique_string_list(&self.allowed_tools, "capabilityGrant.allowedTools")
    }
}

/// `spawn` 的稳定调用参数。
///
/// 该 DTO 下沉到 core，是为了让工具层和执行装配层共享同一份参数语义，
/// 避免 `runtime-execution` 只为了复用字段定义而反向依赖 `runtime-agent-tool`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpawnAgentParams {
    /// Agent profile 标识。留空默认 "explore"。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,

    /// 短摘要，给 UI / 标题 / 日志展示用。不参与任务语义。
    pub description: String,

    /// 任务正文。子 Agent 收到的指令主体。必填。
    pub prompt: String,

    /// 可选补充材料。不保证完整历史，只是附加信息。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,

    /// 本次任务级 capability grant。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_grant: Option<SpawnCapabilityGrant>,
}

impl SpawnAgentParams {
    /// 校验参数合法性。
    pub fn validate(&self) -> Result<()> {
        // prompt 是子 Agent 收到的指令主体，不能为空；
        // 否则 runtime 只能启动一个没有任务语义的空会话。
        require_non_empty_trimmed("prompt", &self.prompt)?;
        // description 只承担可观测性职责；
        // 允许空串兼容模型输出，但纯空白会污染标题与日志。
        require_not_whitespace_only("description", &self.description)?;
        if let Some(grant) = &self.capability_grant {
            grant.validate()?;
        }
        Ok(())
    }
}

/// 子会话事件写入的存储模式。
///
/// TODO: 当前只有 `IndependentSession` 一个变体。
/// 如果未来真的要支持共享 session / 嵌套持久化域等模式，再扩展枚举；
/// 在那之前保留 enum 形状，避免过早把潜在扩展点压成单态值对象。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SubRunStorageMode {
    /// 使用独立 child session。
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

/// 子执行失败分类。
///
/// 这里使用稳定枚举而不是裸字符串，避免前后端各自维护一套错误码字面量。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubRunFailureCode {
    Transport,
    ProviderHttp,
    StreamParse,
    Interrupted,
    Internal,
}

/// child -> parent 的 typed delivery 分类。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParentDeliveryKind {
    Progress,
    Completed,
    Failed,
    CloseRequest,
}

/// child -> parent delivery 的来源。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParentDeliveryOrigin {
    Explicit,
    Fallback,
}

/// delivery 是否终结当前 child work turn。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParentDeliveryTerminalSemantics {
    NonTerminal,
    Terminal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProgressParentDeliveryPayload {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompletedParentDeliveryPayload {
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FailedParentDeliveryPayload {
    pub message: String,
    pub code: SubRunFailureCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub technical_message: Option<String>,
    pub retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CloseRequestParentDeliveryPayload {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// child -> parent 的结构化 payload。
///
/// 使用判别联合而不是无结构 blob，防止 contract 退化回“只有 kind + 文本”。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum ParentDeliveryPayload {
    Progress(ProgressParentDeliveryPayload),
    Completed(CompletedParentDeliveryPayload),
    Failed(FailedParentDeliveryPayload),
    CloseRequest(CloseRequestParentDeliveryPayload),
}

impl ParentDeliveryPayload {
    pub fn kind(&self) -> ParentDeliveryKind {
        match self {
            Self::Progress(_) => ParentDeliveryKind::Progress,
            Self::Completed(_) => ParentDeliveryKind::Completed,
            Self::Failed(_) => ParentDeliveryKind::Failed,
            Self::CloseRequest(_) => ParentDeliveryKind::CloseRequest,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Progress(payload) => payload.message.as_str(),
            Self::Completed(payload) => payload.message.as_str(),
            Self::Failed(payload) => payload.message.as_str(),
            Self::CloseRequest(payload) => payload.message.as_str(),
        }
    }
}

/// child -> parent 的 typed delivery envelope。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ParentDelivery {
    pub idempotency_key: String,
    pub origin: ParentDeliveryOrigin,
    pub terminal_semantics: ParentDeliveryTerminalSemantics,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_turn_id: Option<String>,
    #[serde(flatten)]
    pub payload: ParentDeliveryPayload,
}

/// 子执行传递给父会话的业务结果。
///
/// 该结构只承载“父 Agent 后续决策真正需要消费的内容”，
/// 明确排除 transport/provider/internal diagnostics。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SubRunHandoff {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<ParentDelivery>,
}

/// 子执行失败的结构化信息。
///
/// `display_message` 面向父 Agent / UI 主视图，要求短且稳定；
/// `technical_message` 仅用于调试与次级展示，不应直接进入父会话 handoff。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubRunFailure {
    pub code: SubRunFailureCode,
    pub display_message: String,
    pub technical_message: String,
    pub retryable: bool,
}

use lifecycle::AgentLifecycleStatus;

/// 子执行结构化结果。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompletedSubRunOutcome {
    Completed,
    TokenExceeded,
}

impl CompletedSubRunOutcome {
    pub fn as_turn_outcome(self) -> lifecycle::AgentTurnOutcome {
        match self {
            Self::Completed => lifecycle::AgentTurnOutcome::Completed,
            Self::TokenExceeded => lifecycle::AgentTurnOutcome::TokenExceeded,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailedSubRunOutcome {
    Failed,
    Cancelled,
}

impl FailedSubRunOutcome {
    pub fn as_turn_outcome(self) -> lifecycle::AgentTurnOutcome {
        match self {
            Self::Failed => lifecycle::AgentTurnOutcome::Failed,
            Self::Cancelled => lifecycle::AgentTurnOutcome::Cancelled,
        }
    }
}

/// 子执行对外可观察的正式状态。
///
/// 这是 `SubRunResult` 的 canonical status projection，避免外围再组合
/// `lifecycle + last_turn_outcome` 反推业务语义。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubRunStatus {
    Running,
    Completed,
    TokenExceeded,
    Failed,
    Cancelled,
}

impl SubRunStatus {
    pub fn lifecycle(self) -> AgentLifecycleStatus {
        match self {
            Self::Running => AgentLifecycleStatus::Running,
            Self::Completed | Self::TokenExceeded | Self::Failed | Self::Cancelled => {
                AgentLifecycleStatus::Idle
            },
        }
    }

    pub fn last_turn_outcome(self) -> Option<lifecycle::AgentTurnOutcome> {
        match self {
            Self::Running => None,
            Self::Completed => Some(lifecycle::AgentTurnOutcome::Completed),
            Self::TokenExceeded => Some(lifecycle::AgentTurnOutcome::TokenExceeded),
            Self::Failed => Some(lifecycle::AgentTurnOutcome::Failed),
            Self::Cancelled => Some(lifecycle::AgentTurnOutcome::Cancelled),
        }
    }

    pub fn is_failed(self) -> bool {
        matches!(self, Self::Failed)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::TokenExceeded => "token_exceeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SubRunResult {
    Running {
        handoff: SubRunHandoff,
    },
    Completed {
        outcome: CompletedSubRunOutcome,
        handoff: SubRunHandoff,
    },
    Failed {
        outcome: FailedSubRunOutcome,
        failure: SubRunFailure,
    },
}

impl SubRunResult {
    pub fn status(&self) -> SubRunStatus {
        match self {
            Self::Running { .. } => SubRunStatus::Running,
            Self::Completed { outcome, .. } => match outcome {
                CompletedSubRunOutcome::Completed => SubRunStatus::Completed,
                CompletedSubRunOutcome::TokenExceeded => SubRunStatus::TokenExceeded,
            },
            Self::Failed { outcome, .. } => match outcome {
                FailedSubRunOutcome::Failed => SubRunStatus::Failed,
                FailedSubRunOutcome::Cancelled => SubRunStatus::Cancelled,
            },
        }
    }

    pub fn lifecycle(&self) -> AgentLifecycleStatus {
        self.status().lifecycle()
    }

    pub fn last_turn_outcome(&self) -> Option<lifecycle::AgentTurnOutcome> {
        self.status().last_turn_outcome()
    }

    pub fn handoff(&self) -> Option<&SubRunHandoff> {
        match self {
            Self::Running { handoff } | Self::Completed { handoff, .. } => Some(handoff),
            Self::Failed { .. } => None,
        }
    }

    pub fn failure(&self) -> Option<&SubRunFailure> {
        match self {
            Self::Failed { failure, .. } => Some(failure),
            Self::Running { .. } | Self::Completed { .. } => None,
        }
    }

    pub fn is_failed(&self) -> bool {
        self.status().is_failed()
    }
}

/// 调用侧可传入的子会话上下文 override。
///
/// 使用 `Option` 字段而不是硬编码完整配置，原因是调用方通常只覆写极少数字段；
/// 其余维度应继续沿用 runtime 的默认强隔离策略。
///
/// ## 当前约束
///
/// 以下字段有运行时限制，不是所有值都支持：
///
/// - `inherit_cancel_token`: 不支持设为 `false`。原因是取消必须级联传播， 否则父 Agent 取消后子
///   Agent 会成为孤儿进程继续运行，造成资源泄漏。 TODO: 未来可考虑实现独立的子 Agent
///   超时机制，允许有限度的取消隔离。
///
/// - `include_recovery_refs`: 不支持设为 `true`。恢复引用涉及复杂的跨会话状态依赖， 当前子 Agent
///   执行模型不保证这些引用在子会话中仍然有效。 TODO: 需要先设计跨会话引用的稳定协议后才能开放。
///
/// - `include_parent_findings`: 不支持设为 `true`。父 Agent 的 findings 是非结构化的，
///   直接注入可能导致上下文污染或意外行为。 TODO: 需要先定义 findings 的结构化格式和过滤机制。
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
    /// 取消令牌继承。**不支持设为 false**，见结构体文档说明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_cancel_token: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_compact_summary: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_recent_tail: Option<bool>,
    /// 恢复引用包含。**不支持设为 true**，见结构体文档说明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_recovery_refs: Option<bool>,
    /// 父 Agent findings 包含。**不支持设为 true**，见结构体文档说明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_parent_findings: Option<bool>,
    /// Fork 上下文继承模式。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_mode: Option<ForkMode>,
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
    pub fork_mode: Option<ForkMode>,
}

impl Default for ResolvedSubagentContextOverrides {
    fn default() -> Self {
        Self {
            // 默认始终使用独立子会话模式。
            storage_mode: SubRunStorageMode::IndependentSession,
            inherit_system_instructions: true,
            inherit_project_instructions: true,
            inherit_working_dir: true,
            inherit_policy_upper_bound: true,
            inherit_cancel_token: true,
            include_compact_summary: false,
            include_recent_tail: true,
            include_recovery_refs: false,
            include_parent_findings: false,
            fork_mode: None,
        }
    }
}

/// 解析后的执行限制快照。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedExecutionLimitsSnapshot {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_steps: Option<u32>,
}

/// child delegation 的轻量元数据。
///
/// 这是 launch / resume / observe 共用的责任连续性投影，
/// 用来描述“这个 child 负责哪条责任分支”以及“复用时要遵守什么边界”。
/// 它不是新的 durable 真相，真正事实仍然来自 lifecycle / turn outcome /
/// resolved capability surface。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DelegationMetadata {
    pub responsibility_summary: String,
    pub reuse_scope_summary: String,
    #[serde(default)]
    pub restricted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_limit_summary: Option<String>,
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
    /// 模型偏好。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_preference: Option<String>,
}

/// 子 Agent profile 目录抽象。
///
/// prompt 组装和执行装配都需要读取当前运行时可见的子 Agent 列表，
/// 因此该 discovery 契约应属于 core 边界，而不是某个具体 tool crate。
pub trait AgentProfileCatalog: Send + Sync {
    fn list_subagent_profiles(&self) -> Vec<AgentProfile>;
}

/// 受控子会话的轻量运行句柄。
///
/// 这是 subrun 运行时句柄与 lineage 核心事实的唯一 owner。
/// 所有 lineage 信息直接从此结构读取，不再通过额外的 descriptor 对象。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubRunHandle {
    /// 稳定的子执行域 ID。
    pub sub_run_id: SubRunId,
    /// 运行时分配的 agent 实例 ID。
    pub agent_id: AgentId,
    /// 子会话写入所在的 session。
    pub session_id: SessionId,
    /// 若使用独立子会话，这里记录 child session id。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<SessionId>,
    /// 当前子 Agent 在父子树中的深度。
    pub depth: usize,
    /// 触发该子会话的父 turn。必填：lineage 核心事实，不为 downgrade 保持 optional。
    pub parent_turn_id: TurnId,
    /// 触发该子会话的父 agent。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<AgentId>,
    /// 触发该子会话的父 sub-run。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_sub_run_id: Option<SubRunId>,
    /// 当前执行实例的谱系来源。
    #[serde(default = "default_child_session_lineage_kind")]
    pub lineage_kind: ChildSessionLineageKind,
    /// 该实例绑定的 profile ID。
    pub agent_profile: String,
    /// 当前存储模式。
    pub storage_mode: SubRunStorageMode,
    /// 当前生命周期状态。
    pub lifecycle: AgentLifecycleStatus,
    /// 最近一轮执行的结束原因。Running/Pending 期间为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_turn_outcome: Option<lifecycle::AgentTurnOutcome>,
    /// 当前 agent 执行实例生效的 capability 限制快照。
    #[serde(default)]
    pub resolved_limits: ResolvedExecutionLimitsSnapshot,
    /// 当前 child 责任分支与复用边界的轻量元数据。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation: Option<DelegationMetadata>,
}

impl SubRunHandle {
    pub fn child_identity(&self) -> ChildExecutionIdentity {
        ChildExecutionIdentity {
            agent_id: self.agent_id.clone(),
            session_id: self.session_id.clone(),
            sub_run_id: self.sub_run_id.clone(),
        }
    }

    pub fn parent_ref(&self) -> ParentExecutionRef {
        ParentExecutionRef {
            parent_agent_id: self.parent_agent_id.clone(),
            parent_sub_run_id: self.parent_sub_run_id.clone(),
        }
    }

    pub fn open_session_id(&self) -> SessionId {
        self.child_session_id
            .clone()
            .unwrap_or_else(|| self.session_id.clone())
    }

    pub fn child_ref(&self) -> ChildAgentRef {
        self.child_ref_with_status(self.lifecycle)
    }

    pub fn child_ref_with_status(&self, status: AgentLifecycleStatus) -> ChildAgentRef {
        ChildAgentRef {
            identity: self.child_identity(),
            parent: self.parent_ref(),
            lineage_kind: self.lineage_kind,
            status,
            open_session_id: self.open_session_id(),
        }
    }
}

/// 子会话 lineage 来源。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChildSessionLineageKind {
    Spawn,
    Fork,
    Resume,
}

fn default_child_session_lineage_kind() -> ChildSessionLineageKind {
    ChildSessionLineageKind::Spawn
}

/// 子会话状态来源。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ChildSessionStatusSource {
    Live,
    Durable,
}

/// 共享的 child execution identity。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChildExecutionIdentity {
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub sub_run_id: SubRunId,
}

/// 共享的 parent lineage 指针。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ParentExecutionRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_sub_run_id: Option<SubRunId>,
}

/// 父/子协作面暴露的稳定子会话引用。
///
/// 只承载 child identity、lineage、status 和唯一 canonical open target。
/// "是否可打开"由 `open_session_id` 是否存在来判断，不再通过 duplicated bool。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChildAgentRef {
    #[serde(flatten)]
    pub identity: ChildExecutionIdentity,
    #[serde(flatten)]
    pub parent: ParentExecutionRef,
    pub lineage_kind: ChildSessionLineageKind,
    pub status: AgentLifecycleStatus,
    /// 唯一 canonical child open target。通知、DTO 与其他外层载荷不得重复持有同值字段。
    pub open_session_id: SessionId,
}

impl ChildAgentRef {
    pub fn agent_id(&self) -> &AgentId {
        &self.identity.agent_id
    }

    pub fn session_id(&self) -> &SessionId {
        &self.identity.session_id
    }

    pub fn sub_run_id(&self) -> &SubRunId {
        &self.identity.sub_run_id
    }

    pub fn parent_agent_id(&self) -> Option<&AgentId> {
        self.parent.parent_agent_id.as_ref()
    }

    pub fn parent_sub_run_id(&self) -> Option<&SubRunId> {
        self.parent.parent_sub_run_id.as_ref()
    }

    pub fn to_child_session_node(
        &self,
        parent_turn_id: TurnId,
        status_source: ChildSessionStatusSource,
        created_by_tool_call_id: Option<DeliveryId>,
        lineage_snapshot: Option<LineageSnapshot>,
    ) -> ChildSessionNode {
        ChildSessionNode {
            identity: self.identity.clone(),
            child_session_id: self.open_session_id.clone(),
            parent_session_id: self.session_id().clone(),
            parent: self.parent.clone(),
            parent_turn_id,
            lineage_kind: self.lineage_kind,
            status: self.status,
            status_source,
            created_by_tool_call_id,
            lineage_snapshot,
        }
    }
}

/// 子会话 lineage 快照元数据。
///
/// 记录创建子会话时的谱系来源上下文，
/// fork 时记录源 agent/session，resume 时记录原始 agent/session。
/// spawn 时为 None（没有来源上下文）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LineageSnapshot {
    /// 谱系来源 agent ID（fork 时为源 agent，resume 时为原始 agent）。
    pub source_agent_id: AgentId,
    /// 谱系来源 session ID。
    pub source_session_id: SessionId,
    /// 谱系来源 sub_run_id（如果适用）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_sub_run_id: Option<SubRunId>,
}

/// durable 子会话节点。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChildSessionNode {
    #[serde(flatten)]
    pub identity: ChildExecutionIdentity,
    pub child_session_id: SessionId,
    pub parent_session_id: SessionId,
    #[serde(flatten)]
    pub parent: ParentExecutionRef,
    pub parent_turn_id: TurnId,
    pub lineage_kind: ChildSessionLineageKind,
    pub status: AgentLifecycleStatus,
    pub status_source: ChildSessionStatusSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_tool_call_id: Option<DeliveryId>,
    /// 谱系来源快照。fork/resume 时记录来源上下文，spawn 时为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_snapshot: Option<LineageSnapshot>,
}

impl ChildSessionNode {
    pub fn agent_id(&self) -> &AgentId {
        &self.identity.agent_id
    }

    pub fn session_id(&self) -> &SessionId {
        &self.identity.session_id
    }

    pub fn sub_run_id(&self) -> &SubRunId {
        &self.identity.sub_run_id
    }

    pub fn parent_agent_id(&self) -> Option<&AgentId> {
        self.parent.parent_agent_id.as_ref()
    }

    pub fn parent_sub_run_id(&self) -> Option<&SubRunId> {
        self.parent.parent_sub_run_id.as_ref()
    }

    /// 将 durable 节点转换为可返回给调用方的稳定 child ref。
    ///
    /// 只返回正式 child 事实，不注入额外 UI 派生值。
    pub fn child_ref(&self) -> ChildAgentRef {
        ChildAgentRef {
            identity: self.identity.clone(),
            parent: self.parent.clone(),
            lineage_kind: self.lineage_kind,
            status: self.status,
            open_session_id: self.child_session_id.clone(),
        }
    }
}

/// 父会话可消费的 child-session 通知类型。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChildSessionNotificationKind {
    Started,
    ProgressSummary,
    Delivered,
    Waiting,
    Resumed,
    Closed,
    Failed,
}

/// durable 子会话通知。
///
/// open target 统一从 `child_ref.open_session_id` 读取，不再在外层重复存放。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct ChildSessionNotification {
    pub notification_id: DeliveryId,
    pub child_ref: ChildAgentRef,
    pub kind: ChildSessionNotificationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_tool_call_id: Option<DeliveryId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<ParentDelivery>,
}

/// `send` 的稳定调用参数。
///
/// 统一承载 parent -> child 与 child -> direct parent 两个方向的协作消息。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SendToChildParams {
    /// 目标子 Agent 的稳定 ID。
    pub agent_id: AgentId,
    /// 追加给子 Agent 的消息内容。
    pub message: String,
    /// 可选补充上下文。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

impl SendToChildParams {
    pub fn validate(&self) -> Result<()> {
        require_non_empty_trimmed("agentId", &self.agent_id)?;
        require_non_empty_trimmed("message", &self.message)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SendToParentParams {
    #[serde(flatten)]
    pub payload: ParentDeliveryPayload,
}

impl SendToParentParams {
    pub fn validate(&self) -> Result<()> {
        require_non_empty_trimmed("message", self.payload.message())?;
        Ok(())
    }
}

/// `send` 的稳定调用参数。
///
/// 通过显式方向标记承载下行委派和上行交付，避免 untagged 反序列化歧义。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "direction", rename_all = "snake_case")]
pub enum SendAgentParams {
    #[serde(rename = "child")]
    ToChild(SendToChildParams),
    #[serde(rename = "parent")]
    ToParent(SendToParentParams),
}

impl SendAgentParams {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::ToChild(params) => params.validate(),
            Self::ToParent(params) => params.validate(),
        }
    }
}

/// `close` 的稳定调用参数。
///
/// 关闭指定 child agent 及其子树。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CloseAgentParams {
    /// 目标子 Agent 的稳定 ID。
    pub agent_id: AgentId,
}

impl CloseAgentParams {
    /// 校验参数合法性。
    pub fn validate(&self) -> Result<()> {
        require_non_empty_trimmed("agentId", &self.agent_id)?;
        Ok(())
    }
}

/// 协作工具的统一执行结果。
///
/// 结果本身携带动作语义，避免再额外维护一套并行 kind + option 矩阵。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CollaborationResult {
    Sent {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        continuation: Option<crate::ExecutionContinuation>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delivery_id: Option<DeliveryId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delegation: Option<DelegationMetadata>,
    },
    Observed {
        continuation: crate::ExecutionContinuation,
        summary: String,
        observe_result: Box<input_queue::ObserveSnapshot>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delegation: Option<DelegationMetadata>,
    },
    Closed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        continuation: Option<crate::ExecutionContinuation>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        cascade: bool,
        closed_root_agent_id: AgentId,
    },
}

impl CollaborationResult {
    pub fn continuation(&self) -> Option<&crate::ExecutionContinuation> {
        match self {
            Self::Sent { continuation, .. } => continuation.as_ref(),
            Self::Observed { continuation, .. } => Some(continuation),
            Self::Closed { continuation, .. } => continuation.as_ref(),
        }
    }

    pub fn child_agent_ref(&self) -> Option<&ChildAgentRef> {
        self.continuation()
            .and_then(crate::ExecutionContinuation::child_agent_ref)
    }

    pub fn delivery_id(&self) -> Option<&DeliveryId> {
        match self {
            Self::Sent { delivery_id, .. } => delivery_id.as_ref(),
            Self::Observed { .. } | Self::Closed { .. } => None,
        }
    }

    pub fn summary(&self) -> Option<&str> {
        match self {
            Self::Sent { summary, .. } => summary.as_deref(),
            Self::Observed { summary, .. } => Some(summary.as_str()),
            Self::Closed { summary, .. } => summary.as_deref(),
        }
    }

    pub fn observe_result(&self) -> Option<&input_queue::ObserveSnapshot> {
        match self {
            Self::Observed { observe_result, .. } => Some(observe_result.as_ref()),
            Self::Sent { .. } | Self::Closed { .. } => None,
        }
    }

    pub fn delegation(&self) -> Option<&DelegationMetadata> {
        match self {
            Self::Sent { delegation, .. } | Self::Observed { delegation, .. } => {
                delegation.as_ref()
            },
            Self::Closed { .. } => None,
        }
    }

    pub fn cascade(&self) -> Option<bool> {
        match self {
            Self::Closed { cascade, .. } => Some(*cascade),
            Self::Sent { .. } | Self::Observed { .. } => None,
        }
    }

    pub fn closed_root_agent_id(&self) -> Option<&AgentId> {
        match self {
            Self::Closed {
                closed_root_agent_id,
                ..
            } => Some(closed_root_agent_id),
            Self::Sent { .. } | Self::Observed { .. } => None,
        }
    }
}

/// 协作动作类型。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentCollaborationActionKind {
    Spawn,
    Send,
    Observe,
    Close,
    Delivery,
}

/// 协作动作结果类型。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentCollaborationOutcomeKind {
    Accepted,
    Reused,
    Queued,
    Rejected,
    Failed,
    Delivered,
    Consumed,
    Replayed,
    Closed,
}

/// 记录协作动作发生时的策略上下文。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCollaborationPolicyContext {
    pub policy_revision: String,
    pub max_subrun_depth: usize,
    pub max_spawn_per_turn: usize,
}

/// 结构化协作事实。
///
/// 这是 agent-tool 评估系统的原始事实层；
/// 聚合比率与 scorecard 都应从这些事实推导，而不是反过来改写它。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCollaborationFact {
    pub fact_id: DeliveryId,
    pub action: AgentCollaborationActionKind,
    pub outcome: AgentCollaborationOutcomeKind,
    pub parent_session_id: SessionId,
    pub turn_id: TurnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<AgentId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_identity: Option<ChildExecutionIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_id: Option<DeliveryId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_tool_call_id: Option<DeliveryId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode_id: Option<crate::ModeId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub governance_revision: Option<String>,
    pub policy: AgentCollaborationPolicyContext,
}

impl AgentCollaborationFact {
    pub fn child_agent_id(&self) -> Option<&AgentId> {
        self.child_identity
            .as_ref()
            .map(|identity| &identity.agent_id)
    }

    pub fn child_session_id(&self) -> Option<&SessionId> {
        self.child_identity
            .as_ref()
            .map(|identity| &identity.session_id)
    }

    pub fn child_sub_run_id(&self) -> Option<&SubRunId> {
        self.child_identity
            .as_ref()
            .map(|identity| &identity.sub_run_id)
    }
}

/// Agent 收件箱信封。
///
/// 记录一次协作消息投递（send / 父子交付产出的信封），
/// 包含投递来源、内容和去重标识。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentInboxEnvelope {
    /// 投递唯一 ID，用于幂等去重。
    pub delivery_id: String,
    /// 发送方 agent ID。
    pub from_agent_id: String,
    /// 目标 agent ID。
    pub to_agent_id: String,
    /// 信封类型。
    pub kind: InboxEnvelopeKind,
    /// 消息正文。
    pub message: String,
    /// 可选补充上下文。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// 是否为最终交付（子 agent 交付产出的信封标记为 final）。
    #[serde(default)]
    pub is_final: bool,
    /// 交付摘要（子 agent 交付场景）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// 交付发现列表（子 agent 交付场景）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<String>,
    /// 交付产物引用（子 agent 交付场景）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
}

/// 收件箱信封类型。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InboxEnvelopeKind {
    /// 来自父 agent 的追加消息（send）。
    ParentMessage,
    /// 来自子 agent 的向上交付（子 agent 向父 inbox 投递结果）。
    ChildDelivery,
}

/// turn 级事件的 Agent 元数据。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentEventContext {
    /// 事件所属的 agent 实例 ID。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// 父 turn ID。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_turn_id: Option<TurnId>,
    /// 使用的 profile ID。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_profile: Option<String>,
    /// 受控子会话执行域 ID。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_run_id: Option<SubRunId>,
    /// 父 sub-run ID。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_sub_run_id: Option<SubRunId>,
    /// 执行来源。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_kind: Option<InvocationKind>,
    /// 事件写入模式。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_mode: Option<SubRunStorageMode>,
    /// 独立子会话 ID（若存在）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<SessionId>,
}

impl AgentEventContext {
    /// 构造一个子会话事件上下文。
    pub fn sub_run(
        agent_id: impl Into<AgentId>,
        parent_turn_id: impl Into<TurnId>,
        agent_profile: impl Into<String>,
        sub_run_id: impl Into<SubRunId>,
        parent_sub_run_id: Option<SubRunId>,
        storage_mode: SubRunStorageMode,
        child_session_id: Option<SessionId>,
    ) -> Self {
        let child_session_id = match storage_mode {
            SubRunStorageMode::IndependentSession => {
                let session_id = child_session_id.unwrap_or_else(|| {
                    panic!("IndependentSession sub-run event context requires child_session_id")
                });
                if session_id.trim().is_empty() {
                    panic!(
                        "IndependentSession sub-run event context requires non-empty \
                         child_session_id"
                    );
                }
                Some(session_id)
            },
        };
        Self {
            agent_id: Some(agent_id.into()),
            parent_turn_id: Some(parent_turn_id.into()),
            agent_profile: Some(agent_profile.into()),
            sub_run_id: Some(sub_run_id.into()),
            parent_sub_run_id,
            invocation_kind: Some(InvocationKind::SubRun),
            storage_mode: Some(storage_mode),
            child_session_id,
        }
    }

    /// 为根执行构造事件上下文。
    pub fn root_execution(agent_id: impl Into<AgentId>, agent_profile: impl Into<String>) -> Self {
        Self {
            agent_id: Some(agent_id.into()),
            parent_turn_id: None,
            agent_profile: Some(agent_profile.into()),
            sub_run_id: None,
            parent_sub_run_id: None,
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
            && self.parent_sub_run_id.is_none()
            && self.invocation_kind.is_none()
            && self.storage_mode.is_none()
            && self.child_session_id.is_none()
    }

    /// 判断是否为一个语义完整的独立子会话事件。
    pub fn is_independent_sub_run(&self) -> bool {
        self.invocation_kind == Some(InvocationKind::SubRun)
            && self.storage_mode == Some(SubRunStorageMode::IndependentSession)
            && self
                .child_session_id
                .as_ref()
                .is_some_and(|session_id| !session_id.trim().is_empty())
    }

    /// 判断该事件是否属于指定独立子会话。
    pub fn belongs_to_child_session(&self, session_id: &str) -> bool {
        self.is_independent_sub_run() && self.child_session_id.as_deref() == Some(session_id)
    }

    /// 校验该上下文是否适合作为 durable StorageEvent 的 agent 头部。
    ///
    /// 校验规则：
    /// - RootExecution：必须有 agent_id + agent_profile，不能有任何 sub-run 字段
    /// - SubRun：必须有 agent_id + parent_turn_id + agent_profile + sub_run_id， 且必须是带
    ///   child_session_id 的 IndependentSession
    /// - 非空上下文必须声明 invocation_kind
    pub fn validate_for_storage_event(&self) -> Result<()> {
        if self.is_empty() {
            return Ok(());
        }

        match self.invocation_kind {
            Some(InvocationKind::RootExecution) => {
                if self.agent_id.as_deref().is_none_or(str::is_empty) {
                    return Err(AstrError::Validation(
                        "RootExecution 事件缺少 agent_id".to_string(),
                    ));
                }
                if self.agent_profile.as_deref().is_none_or(str::is_empty) {
                    return Err(AstrError::Validation(
                        "RootExecution 事件缺少 agent_profile".to_string(),
                    ));
                }
                if self.parent_turn_id.is_some()
                    || self.sub_run_id.is_some()
                    || self.parent_sub_run_id.is_some()
                    || self.storage_mode.is_some()
                    || self.child_session_id.is_some()
                {
                    return Err(AstrError::Validation(
                        "RootExecution 事件不允许携带 sub-run 字段".to_string(),
                    ));
                }
                Ok(())
            },
            Some(InvocationKind::SubRun) => {
                if self.agent_id.as_deref().is_none_or(str::is_empty) {
                    return Err(AstrError::Validation(
                        "SubRun 事件缺少 agent_id".to_string(),
                    ));
                }
                if self.parent_turn_id.as_deref().is_none_or(str::is_empty) {
                    return Err(AstrError::Validation(
                        "SubRun 事件缺少 parent_turn_id".to_string(),
                    ));
                }
                if self.agent_profile.as_deref().is_none_or(str::is_empty) {
                    return Err(AstrError::Validation(
                        "SubRun 事件缺少 agent_profile".to_string(),
                    ));
                }
                if self.sub_run_id.as_deref().is_none_or(str::is_empty) {
                    return Err(AstrError::Validation(
                        "SubRun 事件缺少 sub_run_id".to_string(),
                    ));
                }
                if !self.is_independent_sub_run() {
                    return Err(AstrError::Validation(
                        "SubRun 事件必须是带 child_session_id 的 IndependentSession".to_string(),
                    ));
                }
                Ok(())
            },
            None => Err(AstrError::Validation(
                "非空 AgentEventContext 必须声明 invocation_kind".to_string(),
            )),
        }
    }
}

/// 从 SubRunHandle 直接构造事件上下文，替代手工字段拼装。
impl From<&SubRunHandle> for AgentEventContext {
    fn from(handle: &SubRunHandle) -> Self {
        Self {
            agent_id: Some(handle.agent_id.clone()),
            parent_turn_id: Some(handle.parent_turn_id.clone()),
            agent_profile: Some(handle.agent_profile.clone()),
            sub_run_id: Some(handle.sub_run_id.clone()),
            parent_sub_run_id: handle.parent_sub_run_id.clone(),
            invocation_kind: Some(InvocationKind::SubRun),
            storage_mode: Some(handle.storage_mode),
            child_session_id: handle.child_session_id.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AgentLifecycleStatus, ChildExecutionIdentity, ChildSessionLineageKind, ChildSessionNode,
        ChildSessionNotification, ChildSessionStatusSource, ParentExecutionRef, SpawnAgentParams,
        SpawnCapabilityGrant, SubRunHandoff, SubRunStorageMode,
    };
    use crate::{AgentId, DeliveryId, SessionId, SubRunId, TurnId};

    #[test]
    fn spawn_agent_params_reject_empty_prompt() {
        let error = SpawnAgentParams {
            r#type: Some("plan".to_string()),
            description: "review".to_string(),
            prompt: "   ".to_string(),
            context: None,
            capability_grant: None,
        }
        .validate()
        .expect_err("blank prompt should be rejected");

        assert!(error.to_string().contains("prompt 不能为空"));
    }

    #[test]
    fn spawn_agent_params_reject_whitespace_only_description() {
        let error = SpawnAgentParams {
            r#type: Some("plan".to_string()),
            description: " \t ".to_string(),
            prompt: "review".to_string(),
            context: None,
            capability_grant: None,
        }
        .validate()
        .expect_err("whitespace-only description should be rejected");

        assert!(error.to_string().contains("description 不能为纯空白"));
    }

    #[test]
    fn child_session_node_can_build_stable_child_ref() {
        let node = ChildSessionNode {
            identity: ChildExecutionIdentity {
                agent_id: AgentId::from("agent-child"),
                session_id: SessionId::from("session-parent"),
                sub_run_id: SubRunId::from("subrun-1"),
            },
            child_session_id: SessionId::from("session-child"),
            parent_session_id: SessionId::from("session-parent"),
            parent: ParentExecutionRef {
                parent_agent_id: Some(AgentId::from("agent-parent")),
                parent_sub_run_id: Some(SubRunId::from("subrun-parent")),
            },
            parent_turn_id: TurnId::from("turn-parent"),
            lineage_kind: ChildSessionLineageKind::Spawn,
            status: AgentLifecycleStatus::Running,
            status_source: ChildSessionStatusSource::Durable,
            created_by_tool_call_id: Some(DeliveryId::from("call-1")),
            lineage_snapshot: None,
        };

        let child_ref = node.child_ref();

        assert_eq!(child_ref.agent_id().as_str(), "agent-child");
        assert_eq!(child_ref.sub_run_id().as_str(), "subrun-1");
        assert_eq!(child_ref.open_session_id.as_str(), "session-child");
        assert_eq!(
            child_ref.parent_agent_id().map(AgentId::as_str),
            Some("agent-parent")
        );
    }

    #[test]
    fn spawn_capability_grant_rejects_blank_and_duplicate_tools() {
        let error = SpawnCapabilityGrant {
            allowed_tools: vec!["readFile".to_string(), "  ".to_string()],
        }
        .validate()
        .expect_err("blank tool names should be rejected");
        assert!(error.to_string().contains("allowedTools"));

        let error = SpawnCapabilityGrant {
            allowed_tools: vec!["readFile".to_string(), "readFile".to_string()],
        }
        .validate()
        .expect_err("duplicate tool names should be rejected");
        assert!(error.to_string().contains("重复"));

        let error = SpawnCapabilityGrant {
            allowed_tools: Vec::new(),
        }
        .validate()
        .expect_err("empty grants should be rejected");
        assert!(error.to_string().contains("不能为空"));
    }

    #[test]
    #[should_panic(expected = "IndependentSession sub-run event context requires child_session_id")]
    fn sub_run_context_requires_child_session_id_for_independent_session() {
        let _ = super::AgentEventContext::sub_run(
            "agent-child",
            "turn-parent",
            "explore",
            "subrun-1",
            None,
            SubRunStorageMode::IndependentSession,
            None,
        );
    }

    #[test]
    fn subrun_handoff_deserialize_rejects_summary_shape() {
        let handoff = serde_json::from_value::<SubRunHandoff>(serde_json::json!({
            "summary": "removed handoff field",
            "findings": ["done"],
            "artifacts": [],
        }));

        assert!(
            handoff.is_err(),
            "summary-only handoff shape should fail fast"
        );
    }

    #[test]
    fn child_notification_deserialize_rejects_excerpt_shape() {
        let notification = serde_json::from_value::<ChildSessionNotification>(serde_json::json!({
            "notificationId": "delivery-1",
            "childRef": {
                "agentId": "agent-child",
                "sessionId": "session-parent",
                "subRunId": "subrun-child",
                "lineageKind": "spawn",
                "status": "idle",
                "openSessionId": "session-child"
            },
            "kind": "delivered",
            "summary": "removed summary field",
            "finalReplyExcerpt": "removed final field",
            "status": "idle"
        }));

        assert!(
            notification.is_err(),
            "summary/excerpt notification shape should fail fast"
        );
    }

    #[test]
    fn child_notification_deserialize_rejects_failed_shape() {
        let notification = serde_json::from_value::<ChildSessionNotification>(serde_json::json!({
            "notificationId": "delivery-failed",
            "childRef": {
                "agentId": "agent-child",
                "sessionId": "session-parent",
                "subRunId": "subrun-child",
                "lineageKind": "spawn",
                "status": "idle",
                "openSessionId": "session-child"
            },
            "kind": "failed",
            "summary": "removed failure field",
            "status": "idle"
        }));

        assert!(
            notification.is_err(),
            "summary-only failed notification shape should fail fast"
        );
    }

    #[test]
    fn child_notification_deserialize_rejects_closed_shape() {
        let notification = serde_json::from_value::<ChildSessionNotification>(serde_json::json!({
            "notificationId": "delivery-closed",
            "childRef": {
                "agentId": "agent-child",
                "sessionId": "session-parent",
                "subRunId": "subrun-child",
                "lineageKind": "spawn",
                "status": "idle",
                "openSessionId": "session-child"
            },
            "kind": "closed",
            "summary": "removed close-request field",
            "status": "idle"
        }));

        assert!(
            notification.is_err(),
            "summary-only closed notification shape should fail fast"
        );
    }

    #[test]
    fn child_notification_deserialize_rejects_summary_only_progress_shape() {
        let notification = serde_json::from_value::<ChildSessionNotification>(serde_json::json!({
            "notificationId": "delivery-progress",
            "childRef": {
                "agentId": "agent-child",
                "sessionId": "session-parent",
                "subRunId": "subrun-child",
                "lineageKind": "spawn",
                "status": "running",
                "openSessionId": "session-child"
            },
            "kind": "waiting",
            "summary": "removed progress field",
            "status": "running"
        }));

        assert!(
            notification.is_err(),
            "summary-only progress notification shape should fail fast"
        );
    }
}
