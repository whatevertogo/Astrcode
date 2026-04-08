//! # Agent 基础类型
//!
//! 定义 Agent / 子会话控制平面需要复用的稳定 DTO。
//! 这里刻意把“Agent 实例”和“受控子会话执行域”拆开，
//! 这样 runtime、存储事件、SSE 和 UI 都能围绕同一套语义建模。

use serde::{Deserialize, Serialize};

use crate::error::{AstrError, Result};

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

/// Fork 上下文继承模式。
///
/// TODO: 当前仅定义枚举，runtime 侧未完整消费。
/// 未来 compact agent 将使用此字段决定子 agent 继承多少父对话上下文。
/// 参考 Codex 的 SpawnAgentForkMode 设计。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ForkMode {
    /// 继承完整对话历史。
    FullHistory,
    /// 只继承最近 N 轮对话。
    LastNTurns(usize),
}

/// `spawnAgent` 的稳定调用参数。
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
}

impl SpawnAgentParams {
    /// 校验参数合法性。
    pub fn validate(&self) -> Result<()> {
        // prompt 是子 Agent 收到的指令主体，不能为空；
        // 否则 runtime 只能启动一个没有任务语义的空会话。
        if self.prompt.trim().is_empty() {
            return Err(AstrError::Validation("prompt 不能为空".to_string()));
        }
        // description 只承担可观测性职责；
        // 允许空串兼容模型输出，但纯空白会污染标题与日志。
        if !self.description.is_empty() && self.description.trim().is_empty() {
            return Err(AstrError::Validation(
                "description 不能为纯空白".to_string(),
            ));
        }
        Ok(())
    }
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
    Running,
    Completed,
    Failed,
    Aborted,
    TokenExceeded,
}

impl SubRunOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Aborted => "aborted",
            Self::TokenExceeded => "token_exceeded",
        }
    }
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

/// 子执行传递给父会话的业务结果。
///
/// 该结构只承载“父 Agent 后续决策真正需要消费的内容”，
/// 明确排除 transport/provider/internal diagnostics。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubRunHandoff {
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
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

/// 子执行结构化结果。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubRunResult {
    pub status: SubRunOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handoff: Option<SubRunHandoff>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<SubRunFailure>,
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
            storage_mode: SubRunStorageMode::SharedSession,
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
// TODO: 未来可能需要重新添加 max_steps 和 token_budget 参数来限制子智能体执行
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedExecutionLimitsSnapshot {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
}

/// 子执行 lineage 的稳定描述。
///
/// 与运行态 `SubRunHandle` 区分：该结构只承载 durable 事实，不包含状态字段。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubRunDescriptor {
    pub sub_run_id: String,
    pub parent_turn_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<String>,
    pub depth: usize,
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
    // TODO: 未来可能需要重新添加 max_steps 和 token_budget 参数来限制子智能体执行
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

impl SubRunHandle {
    /// 将运行句柄转换为 lineage descriptor（若 parent turn 可用）。
    ///
    /// descriptor 只表达 ownership，不携带运行态 status；
    /// SharedSession/IndependentSession 的差异不会改变该语义。
    pub fn descriptor(&self) -> Option<SubRunDescriptor> {
        self.parent_turn_id
            .as_ref()
            .map(|parent_turn_id| SubRunDescriptor {
                sub_run_id: self.sub_run_id.clone(),
                parent_turn_id: parent_turn_id.clone(),
                parent_agent_id: self.parent_agent_id.clone(),
                depth: self.depth,
            })
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

/// 子会话状态来源。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ChildSessionStatusSource {
    Live,
    Durable,
    LegacyDurable,
}

/// 父/子协作面暴露的稳定子会话引用。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChildAgentRef {
    pub agent_id: String,
    pub session_id: String,
    pub sub_run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<String>,
    pub lineage_kind: ChildSessionLineageKind,
    pub status: AgentStatus,
    pub openable: bool,
    pub open_session_id: String,
}

/// durable 子会话节点。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChildSessionNode {
    pub agent_id: String,
    pub session_id: String,
    pub child_session_id: String,
    pub sub_run_id: String,
    pub parent_session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<String>,
    pub parent_turn_id: String,
    pub lineage_kind: ChildSessionLineageKind,
    pub status: AgentStatus,
    pub status_source: ChildSessionStatusSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_tool_call_id: Option<String>,
}

impl ChildSessionNode {
    /// 将 durable 节点转换为可返回给调用方的稳定 child ref。
    pub fn child_ref(&self) -> ChildAgentRef {
        ChildAgentRef {
            agent_id: self.agent_id.clone(),
            session_id: self.session_id.clone(),
            sub_run_id: self.sub_run_id.clone(),
            parent_agent_id: self.parent_agent_id.clone(),
            lineage_kind: self.lineage_kind,
            status: self.status,
            openable: true,
            open_session_id: self.child_session_id.clone(),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::{
        AgentStatus, ChildSessionLineageKind, ChildSessionNode, ChildSessionStatusSource,
        SpawnAgentParams,
    };

    #[test]
    fn spawn_agent_params_reject_empty_prompt() {
        let error = SpawnAgentParams {
            r#type: Some("plan".to_string()),
            description: "review".to_string(),
            prompt: "   ".to_string(),
            context: None,
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
        }
        .validate()
        .expect_err("whitespace-only description should be rejected");

        assert!(error.to_string().contains("description 不能为纯空白"));
    }

    #[test]
    fn child_session_node_can_build_stable_child_ref() {
        let node = ChildSessionNode {
            agent_id: "agent-child".to_string(),
            session_id: "session-parent".to_string(),
            child_session_id: "session-child".to_string(),
            sub_run_id: "subrun-1".to_string(),
            parent_session_id: "session-parent".to_string(),
            parent_agent_id: Some("agent-parent".to_string()),
            parent_turn_id: "turn-parent".to_string(),
            lineage_kind: ChildSessionLineageKind::Spawn,
            status: AgentStatus::Running,
            status_source: ChildSessionStatusSource::Durable,
            created_by_tool_call_id: Some("call-1".to_string()),
        };

        let child_ref = node.child_ref();

        assert_eq!(child_ref.agent_id, "agent-child");
        assert_eq!(child_ref.sub_run_id, "subrun-1");
        assert_eq!(child_ref.open_session_id, "session-child");
        assert_eq!(child_ref.parent_agent_id.as_deref(), Some("agent-parent"));
        assert!(child_ref.openable);
    }
}
