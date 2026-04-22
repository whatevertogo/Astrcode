use serde::{Deserialize, Serialize};

use super::{
    InvocationKind,
    delivery::{ArtifactRef, ParentDeliveryPayload, SubRunStorageMode},
    input_queue,
    lineage::{ChildAgentRef, ChildExecutionIdentity, SubRunHandle},
    require_non_empty_trimmed,
    spawn::DelegationMetadata,
};
use crate::{
    AgentId, DeliveryId, ExecutionContinuation, ModeId, SessionId, SubRunId, TurnId,
    error::{AstrError, Result},
};

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
        continuation: Option<ExecutionContinuation>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delivery_id: Option<DeliveryId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delegation: Option<DelegationMetadata>,
    },
    Observed {
        continuation: ExecutionContinuation,
        summary: String,
        observe_result: Box<input_queue::ObserveSnapshot>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delegation: Option<DelegationMetadata>,
    },
    Closed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        continuation: Option<ExecutionContinuation>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        cascade: bool,
        closed_root_agent_id: AgentId,
    },
}

impl CollaborationResult {
    pub fn continuation(&self) -> Option<&ExecutionContinuation> {
        match self {
            Self::Sent { continuation, .. } => continuation.as_ref(),
            Self::Observed { continuation, .. } => Some(continuation),
            Self::Closed { continuation, .. } => continuation.as_ref(),
        }
    }

    pub fn child_agent_ref(&self) -> Option<&ChildAgentRef> {
        self.continuation()
            .and_then(ExecutionContinuation::child_agent_ref)
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
    pub mode_id: Option<ModeId>,
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
        AgentEventContext, CloseAgentParams, InvocationKind, SendAgentParams, SendToChildParams,
        SendToParentParams, SubRunStorageMode,
    };
    use crate::{
        ParentDeliveryPayload, ProgressParentDeliveryPayload,
        error::AstrError,
    };

    fn valid_sub_run_context() -> AgentEventContext {
        AgentEventContext {
            agent_id: Some("agent-1".into()),
            parent_turn_id: Some("turn-1".into()),
            agent_profile: Some("default".to_string()),
            sub_run_id: Some("subrun-1".into()),
            parent_sub_run_id: None,
            invocation_kind: Some(InvocationKind::SubRun),
            storage_mode: Some(SubRunStorageMode::IndependentSession),
            child_session_id: Some("child-session-1".into()),
        }
    }

    fn assert_validation_error(ctx: AgentEventContext, expected: &str) {
        let error = ctx
            .validate_for_storage_event()
            .expect_err("context should be rejected");
        assert!(
            error.to_string().contains(expected),
            "unexpected validation error: {error}"
        );
    }

    #[test]
    fn validate_for_storage_event_rejects_non_empty_context_without_invocation_kind() {
        let ctx = AgentEventContext {
            agent_id: Some("agent-1".into()),
            ..Default::default()
        };
        assert_validation_error(ctx, "必须声明 invocation_kind");
    }

    #[test]
    fn validate_for_storage_event_rejects_invalid_root_context() {
        let mut missing_agent = AgentEventContext::root_execution("agent-1", "default");
        missing_agent.agent_id = None;
        assert_validation_error(missing_agent, "RootExecution 事件缺少 agent_id");

        let mut missing_profile = AgentEventContext::root_execution("agent-1", "default");
        missing_profile.agent_profile = None;
        assert_validation_error(missing_profile, "RootExecution 事件缺少 agent_profile");

        let mut carries_subrun_field = AgentEventContext::root_execution("agent-1", "default");
        carries_subrun_field.sub_run_id = Some("subrun-1".into());
        assert_validation_error(
            carries_subrun_field,
            "RootExecution 事件不允许携带 sub-run 字段",
        );
    }

    #[test]
    fn validate_for_storage_event_rejects_invalid_subrun_context() {
        let mut missing_agent = valid_sub_run_context();
        missing_agent.agent_id = None;
        assert_validation_error(missing_agent, "SubRun 事件缺少 agent_id");

        let mut missing_parent_turn = valid_sub_run_context();
        missing_parent_turn.parent_turn_id = None;
        assert_validation_error(missing_parent_turn, "SubRun 事件缺少 parent_turn_id");

        let mut missing_profile = valid_sub_run_context();
        missing_profile.agent_profile = None;
        assert_validation_error(missing_profile, "SubRun 事件缺少 agent_profile");

        let mut missing_subrun = valid_sub_run_context();
        missing_subrun.sub_run_id = None;
        assert_validation_error(missing_subrun, "SubRun 事件缺少 sub_run_id");

        let mut not_independent = valid_sub_run_context();
        not_independent.child_session_id = None;
        assert_validation_error(
            not_independent,
            "SubRun 事件必须是带 child_session_id 的 IndependentSession",
        );
    }

    #[test]
    fn validate_for_storage_event_accepts_valid_contexts() {
        AgentEventContext::root_execution("agent-1", "default")
            .validate_for_storage_event()
            .expect("valid root context should pass");

        valid_sub_run_context()
            .validate_for_storage_event()
            .expect("valid sub-run context should pass");
    }

    fn assert_param_validation_error(result: crate::error::Result<()>, expected: &str) {
        let AstrError::Validation(message) =
            result.expect_err("params should be rejected")
        else {
            panic!("expected validation error");
        };
        assert!(
            message.contains(expected),
            "unexpected validation error: {message}"
        );
    }

    #[test]
    fn send_to_child_params_validate_rejects_blank_fields() {
        assert_param_validation_error(
            SendToChildParams {
                agent_id: " ".into(),
                message: "hello".to_string(),
                context: None,
            }
            .validate(),
            "agentId",
        );
        assert_param_validation_error(
            SendToChildParams {
                agent_id: "agent-1".into(),
                message: " ".to_string(),
                context: None,
            }
            .validate(),
            "message",
        );
    }

    #[test]
    fn send_to_parent_and_send_agent_params_validate_delegate_to_payload_message() {
        assert_param_validation_error(
            SendToParentParams {
                payload: ParentDeliveryPayload::Progress(ProgressParentDeliveryPayload {
                    message: " ".to_string(),
                }),
            }
            .validate(),
            "message",
        );

        assert_param_validation_error(
            SendAgentParams::ToChild(SendToChildParams {
                agent_id: "agent-1".into(),
                message: " ".to_string(),
                context: None,
            })
            .validate(),
            "message",
        );

        SendAgentParams::ToParent(SendToParentParams {
            payload: ParentDeliveryPayload::Progress(ProgressParentDeliveryPayload {
                message: "progress".to_string(),
            }),
        })
        .validate()
        .expect("valid parent payload should pass");
    }

    #[test]
    fn close_agent_params_validate_rejects_blank_agent_id() {
        assert_param_validation_error(
            CloseAgentParams {
                agent_id: " ".into(),
            }
            .validate(),
            "agentId",
        );
    }
}
