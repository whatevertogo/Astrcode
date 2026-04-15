use astrcode_core::{
    ChildSessionNode, DeleteProjectResult, ExecutionAccepted, ResolvedRuntimeConfig, SessionId,
    SessionMeta, StoredEvent,
};
use astrcode_session_runtime::{
    AgentPromptSubmission, SessionCatalogEvent, SessionControlStateSnapshot, SessionReplay,
    SessionRuntime, SessionTranscriptSnapshot,
};
use async_trait::async_trait;
use tokio::sync::broadcast;

/// `App` 依赖的 session-runtime 稳定端口。
///
/// Why: `App` 只编排 session 用例，不应直接耦合 `SessionRuntime` 的具体结构。
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
        submission: AgentPromptSubmission,
    ) -> astrcode_core::Result<ExecutionAccepted>;
    async fn interrupt_session(&self, session_id: &str) -> astrcode_core::Result<()>;
    async fn compact_session(
        &self,
        session_id: &str,
        runtime: ResolvedRuntimeConfig,
    ) -> astrcode_core::Result<bool>;
    async fn session_transcript_snapshot(
        &self,
        session_id: &str,
    ) -> astrcode_core::Result<SessionTranscriptSnapshot>;
    async fn session_control_state(
        &self,
        session_id: &str,
    ) -> astrcode_core::Result<SessionControlStateSnapshot>;
    async fn session_child_nodes(
        &self,
        session_id: &str,
    ) -> astrcode_core::Result<Vec<ChildSessionNode>>;
    async fn session_stored_events(
        &self,
        session_id: &SessionId,
    ) -> astrcode_core::Result<Vec<StoredEvent>>;
    async fn session_replay(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> astrcode_core::Result<SessionReplay>;
}

#[async_trait]
impl AppSessionPort for SessionRuntime {
    fn subscribe_catalog_events(&self) -> broadcast::Receiver<SessionCatalogEvent> {
        self.subscribe_catalog_events()
    }

    async fn list_session_metas(&self) -> astrcode_core::Result<Vec<SessionMeta>> {
        self.list_session_metas().await
    }

    async fn create_session(&self, working_dir: String) -> astrcode_core::Result<SessionMeta> {
        self.create_session(working_dir).await
    }

    async fn delete_session(&self, session_id: &str) -> astrcode_core::Result<()> {
        self.delete_session(session_id).await
    }

    async fn delete_project(
        &self,
        working_dir: &str,
    ) -> astrcode_core::Result<DeleteProjectResult> {
        self.delete_project(working_dir).await
    }

    async fn get_session_working_dir(&self, session_id: &str) -> astrcode_core::Result<String> {
        self.get_session_working_dir(session_id).await
    }

    async fn submit_prompt_for_agent(
        &self,
        session_id: &str,
        text: String,
        runtime: ResolvedRuntimeConfig,
        submission: AgentPromptSubmission,
    ) -> astrcode_core::Result<ExecutionAccepted> {
        self.submit_prompt_for_agent(session_id, text, runtime, submission)
            .await
    }

    async fn interrupt_session(&self, session_id: &str) -> astrcode_core::Result<()> {
        self.interrupt_session(session_id).await
    }

    async fn compact_session(
        &self,
        session_id: &str,
        runtime: ResolvedRuntimeConfig,
    ) -> astrcode_core::Result<bool> {
        self.compact_session(session_id, runtime).await
    }

    async fn session_transcript_snapshot(
        &self,
        session_id: &str,
    ) -> astrcode_core::Result<SessionTranscriptSnapshot> {
        self.session_transcript_snapshot(session_id).await
    }

    async fn session_control_state(
        &self,
        session_id: &str,
    ) -> astrcode_core::Result<SessionControlStateSnapshot> {
        self.session_control_state(session_id).await
    }

    async fn session_child_nodes(
        &self,
        session_id: &str,
    ) -> astrcode_core::Result<Vec<ChildSessionNode>> {
        self.session_child_nodes(session_id).await
    }

    async fn session_stored_events(
        &self,
        session_id: &SessionId,
    ) -> astrcode_core::Result<Vec<StoredEvent>> {
        self.replay_stored_events(session_id).await
    }

    async fn session_replay(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> astrcode_core::Result<SessionReplay> {
        self.session_replay(session_id, last_event_id).await
    }
}
