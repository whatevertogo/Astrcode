//! `App` 依赖的 session 稳定端口。
//!
//! 定义 `AppSessionPort` trait，将应用层与底层 session owner 具体实现解耦。
//! `App` 只编排 session 用例（创建、提交、快照、compact 等），
//! 不直接耦合具体 owner 的 catalog/fork/query helper。

use astrcode_core::{
    ChildSessionNode, DeleteProjectResult, ResolvedRuntimeConfig, SessionMeta, StoredEvent,
    TaskSnapshot,
};
use astrcode_core::mode::ModeId;
use astrcode_host_session::{SessionCatalogEvent, SessionControlStateSnapshot, SessionModeState};
use astrcode_runtime_contract::ExecutionSubmissionOutcome;
use async_trait::async_trait;
use tokio::sync::broadcast;

use super::{AppAgentPromptSubmission, DurableSubRunStatusSummary};
use crate::conversation_read_model::{
    ConversationSnapshotFacts, ConversationStreamReplayFacts, SessionReplay,
    SessionTranscriptSnapshot,
};

/// `App` 依赖的 session 稳定端口。
///
/// Why: `App` 只编排 session 用例，不应直接耦合具体 session owner 的 helper 类型。
#[allow(dead_code)]
#[async_trait]
pub trait AppSessionPort: Send + Sync {
    fn subscribe_catalog_events(&self) -> broadcast::Receiver<SessionCatalogEvent>;

    async fn list_session_metas(&self) -> astrcode_core::Result<Vec<SessionMeta>>;
    async fn create_session(&self, working_dir: String) -> astrcode_core::Result<SessionMeta>;
    async fn delete_session(&self, session_id: &str) -> astrcode_core::Result<()>;
    async fn delete_project(&self, working_dir: &str)
    -> astrcode_core::Result<DeleteProjectResult>;
    async fn get_session_working_dir(&self, session_id: &str) -> astrcode_core::Result<String>;
    async fn submit_prompt_for_agent(
        &self,
        session_id: &str,
        text: String,
        runtime: ResolvedRuntimeConfig,
        submission: AppAgentPromptSubmission,
    ) -> astrcode_core::Result<ExecutionSubmissionOutcome>;
    async fn interrupt_session(&self, session_id: &str) -> astrcode_core::Result<()>;
    async fn compact_session(
        &self,
        session_id: &str,
        runtime: ResolvedRuntimeConfig,
        instructions: Option<String>,
    ) -> astrcode_core::Result<bool>;
    async fn session_transcript_snapshot(
        &self,
        session_id: &str,
    ) -> astrcode_core::Result<SessionTranscriptSnapshot>;
    async fn conversation_snapshot(
        &self,
        session_id: &str,
    ) -> astrcode_core::Result<ConversationSnapshotFacts>;
    async fn session_control_state(
        &self,
        session_id: &str,
    ) -> astrcode_core::Result<SessionControlStateSnapshot>;
    async fn active_task_snapshot(
        &self,
        session_id: &str,
        owner: &str,
    ) -> astrcode_core::Result<Option<TaskSnapshot>>;
    async fn session_mode_state(&self, session_id: &str)
    -> astrcode_core::Result<SessionModeState>;
    async fn switch_mode(
        &self,
        session_id: &str,
        from: ModeId,
        to: ModeId,
    ) -> astrcode_core::Result<StoredEvent>;
    async fn session_child_nodes(
        &self,
        session_id: &str,
    ) -> astrcode_core::Result<Vec<ChildSessionNode>>;
    async fn session_stored_events(
        &self,
        session_id: &str,
    ) -> astrcode_core::Result<Vec<StoredEvent>>;
    async fn durable_subrun_status_snapshot(
        &self,
        parent_session_id: &str,
        requested_subrun_id: &str,
    ) -> astrcode_core::Result<Option<DurableSubRunStatusSummary>>;
    async fn session_replay(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> astrcode_core::Result<SessionReplay>;
    async fn conversation_stream_replay(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> astrcode_core::Result<ConversationStreamReplayFacts>;
}
