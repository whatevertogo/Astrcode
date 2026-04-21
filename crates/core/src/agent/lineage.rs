use serde::{Deserialize, Serialize};

use super::{
    delivery::{ParentDelivery, SubRunStorageMode},
    lifecycle::{AgentLifecycleStatus, AgentTurnOutcome},
    spawn::{DelegationMetadata, ResolvedExecutionLimitsSnapshot},
};
use crate::{AgentId, DeliveryId, SessionId, SubRunId, TurnId};

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
    pub last_turn_outcome: Option<AgentTurnOutcome>,
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
