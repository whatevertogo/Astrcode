//! # Agent 基础类型
//!
//! 定义 Agent / 子会话控制平面需要复用的稳定 DTO。
//! 这里刻意把”Agent 实例”和”受控子会话执行域”拆开，
//! 这样 runtime、存储事件、SSE 和 UI 都能围绕同一套语义建模。
//!
//! 子模块划分：
//! - `lifecycle`：AgentLifecycleStatus + AgentTurnOutcome（四工具模型的状态拆层）
//! - `input_queue`：durable input queue 信封、事件载荷、四工具参数和 observe 快照
//! - `spawn`：spawn 参数、上下文继承与 profile 契约
//! - `delivery`：父子交付 payload、sub-run 结果与 durable handoff
//! - `lineage`：child session / sub-run 谱系与 stable ref
//! - `collaboration`：send/close 参数、协作结果、收件箱与事件上下文

pub mod collaboration;
pub mod delivery;
pub mod input_queue;
pub mod lifecycle;
pub mod lineage;
pub mod spawn;

pub use collaboration::{
    AgentCollaborationActionKind, AgentCollaborationFact, AgentCollaborationOutcomeKind,
    AgentCollaborationPolicyContext, AgentEventContext, AgentInboxEnvelope, CloseAgentParams,
    CollaborationResult, InboxEnvelopeKind, SendAgentParams, SendToChildParams, SendToParentParams,
};
pub use delivery::{
    ArtifactRef, CloseRequestParentDeliveryPayload, CompletedParentDeliveryPayload,
    CompletedSubRunOutcome, FailedParentDeliveryPayload, FailedSubRunOutcome, ParentDelivery,
    ParentDeliveryKind, ParentDeliveryOrigin, ParentDeliveryPayload,
    ParentDeliveryTerminalSemantics, ProgressParentDeliveryPayload, SubRunFailure,
    SubRunFailureCode, SubRunHandoff, SubRunResult, SubRunStatus, SubRunStorageMode,
};
pub use lifecycle::{AgentLifecycleStatus, AgentTurnOutcome};
pub use lineage::{
    ChildAgentRef, ChildExecutionIdentity, ChildSessionLineageKind, ChildSessionNode,
    ChildSessionNotification, ChildSessionNotificationKind, ChildSessionStatusSource,
    LineageSnapshot, ParentExecutionRef,
};
use serde::{Deserialize, Serialize};
pub use spawn::{
    AgentProfile, AgentProfileCatalog, DelegationMetadata, ResolvedExecutionLimitsSnapshot,
    ResolvedSubagentContextOverrides, SpawnAgentParams, SubagentContextOverrides,
};

use crate::error::{AstrError, Result};

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

#[cfg(test)]
mod tests {
    use super::{
        AgentLifecycleStatus, ChildExecutionIdentity, ChildSessionLineageKind, ChildSessionNode,
        ChildSessionNotification, ChildSessionStatusSource, ParentExecutionRef, SpawnAgentParams,
        SubRunHandoff, SubRunStorageMode,
    };
    use crate::{AgentId, DeliveryId, SessionId, SubRunId, TurnId};

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
