//! Agent 编排子域依赖的 session 稳定端口。
//!
//! `AgentSessionPort` 继承 `AppSessionPort`，扩展了 agent 协作编排所需的全部 session 操作：
//! child session 建立、prompt 提交（带 turn id）、durable input queue 管理、
//! collaboration fact 追加、observe 快照、turn 终态等待。
//!
//! 先按职责分组在一个端口中表达完整协作流程，未来根据演化决定是否继续瘦身。
//!
//! 生产路径通过 server-owned bridge 实现该端口，避免把底层 runtime 直接暴露成 session owner。

use astrcode_core::{
    AgentCollaborationFact, AgentEventContext, AgentLifecycleStatus, InputBatchAckedPayload,
    InputBatchStartedPayload, InputDiscardedPayload, InputQueuedPayload, ResolvedRuntimeConfig,
    SessionMeta, StoredEvent, TurnId,
};
use astrcode_runtime_contract::ExecutionAccepted;
use async_trait::async_trait;

use super::{
    AppAgentPromptSubmission, AppSessionPort, RecoverableParentDelivery, SessionObserveSnapshot,
    SessionTurnOutcomeSummary, SessionTurnTerminalState,
};

/// Agent 编排子域依赖的 session 稳定端口。
///
/// Why: 这里的方法虽然不少，但调用者仍是同一批 agent collaboration use case。
/// 先按职责分组，保持一个端口表达完整协作流程，再根据未来演化决定是否继续瘦身。
#[async_trait]
pub trait AgentSessionPort: AppSessionPort {
    // 子 agent session 建立与 prompt 提交。
    async fn create_child_session(
        &self,
        working_dir: &str,
        parent_session_id: &str,
    ) -> astrcode_core::Result<SessionMeta>;
    async fn submit_prompt_for_agent_with_submission(
        &self,
        session_id: &str,
        text: String,
        runtime: ResolvedRuntimeConfig,
        submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<ExecutionAccepted>;
    async fn try_submit_prompt_for_agent_with_turn_id(
        &self,
        session_id: &str,
        turn_id: TurnId,
        text: String,
        runtime: ResolvedRuntimeConfig,
        submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<Option<ExecutionAccepted>>;
    async fn submit_queued_inputs_for_agent_with_turn_id(
        &self,
        session_id: &str,
        turn_id: TurnId,
        queued_inputs: Vec<String>,
        runtime: ResolvedRuntimeConfig,
        submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<Option<ExecutionAccepted>>;

    // Durable input queue / collaboration 事件追加。
    async fn append_agent_input_queued(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: InputQueuedPayload,
    ) -> astrcode_core::Result<StoredEvent>;
    async fn append_agent_input_discarded(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: InputDiscardedPayload,
    ) -> astrcode_core::Result<StoredEvent>;
    async fn append_agent_input_batch_started(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: InputBatchStartedPayload,
    ) -> astrcode_core::Result<StoredEvent>;
    async fn append_agent_input_batch_acked(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        payload: InputBatchAckedPayload,
    ) -> astrcode_core::Result<StoredEvent>;
    async fn append_child_session_notification(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        notification: astrcode_core::ChildSessionNotification,
    ) -> astrcode_core::Result<StoredEvent>;
    async fn append_agent_collaboration_fact(
        &self,
        session_id: &str,
        turn_id: &str,
        agent: AgentEventContext,
        fact: AgentCollaborationFact,
    ) -> astrcode_core::Result<StoredEvent>;
    async fn pending_delivery_ids_for_agent(
        &self,
        session_id: &str,
        agent_id: &str,
    ) -> astrcode_core::Result<Vec<String>>;
    async fn recoverable_parent_deliveries(
        &self,
        parent_session_id: &str,
    ) -> astrcode_core::Result<Vec<RecoverableParentDelivery>>;

    // 观察与投影读取。
    async fn observe_agent_session(
        &self,
        open_session_id: &str,
        target_agent_id: &str,
        lifecycle_status: AgentLifecycleStatus,
    ) -> astrcode_core::Result<SessionObserveSnapshot>;
    async fn project_turn_outcome(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> astrcode_core::Result<SessionTurnOutcomeSummary>;

    // Turn 终态等待。
    async fn wait_for_turn_terminal_snapshot(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> astrcode_core::Result<SessionTurnTerminalState>;
}
