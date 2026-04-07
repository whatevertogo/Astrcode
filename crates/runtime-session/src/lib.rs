//! Session 状态、事件落盘与 turn 生命周期的共享基础设施。
//!
//! 这个 crate 只承载“会话真相”与“单次 turn 执行辅助”，
//! 不承担 RuntimeService 的 façade 组装，也不理解 profile / sub-agent 编排语义。

mod paths;
mod session_state;
mod support;
mod turn_runtime;

use astrcode_core::{AstrError, SessionEventRecord, SessionMeta, SessionTruthBoundary};
use async_trait::async_trait;
pub use paths::{display_name_from_working_dir, normalize_session_id, normalize_working_dir};
pub use session_state::{
    SessionState, SessionStateEventSink, SessionTokenBudgetState, SessionWriter,
};
pub use support::{lock_anyhow, spawn_blocking_anyhow};
pub use turn_runtime::{
    append_and_broadcast, append_and_broadcast_from_turn_callback, complete_session_execution,
    prepare_session_execution, recent_turn_event_tail, should_record_compaction_tail_event,
};

#[async_trait]
pub trait SessionTruthRuntime: Send + Sync {
    async fn create_session(
        &self,
        working_dir: &std::path::Path,
    ) -> std::result::Result<SessionMeta, AstrError>;

    async fn list_sessions(&self) -> std::result::Result<Vec<SessionMeta>, AstrError>;

    async fn load_history(
        &self,
        session_id: &str,
    ) -> std::result::Result<Vec<SessionEventRecord>, AstrError>;
}

/// `runtime-session` 对外暴露的 trait surface。
///
/// 具体 owner 可以由 runtime façade 注入，但 core trait 的实现留在 session crate，
/// 避免边界契约继续散落在上层装配代码里。
#[derive(Clone)]
pub struct SessionTruthSurface<T> {
    inner: T,
}

impl<T> SessionTruthSurface<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    pub fn into_inner(self) -> T {
        self.inner
    }

    pub fn inner(&self) -> &T {
        &self.inner
    }
}

#[async_trait]
impl<T> SessionTruthBoundary for SessionTruthSurface<T>
where
    T: SessionTruthRuntime,
{
    async fn create_session(
        &self,
        working_dir: &std::path::Path,
    ) -> std::result::Result<SessionMeta, AstrError> {
        self.inner.create_session(working_dir).await
    }

    async fn list_sessions(&self) -> std::result::Result<Vec<SessionMeta>, AstrError> {
        self.inner.list_sessions().await
    }

    async fn load_history(
        &self,
        session_id: &str,
    ) -> std::result::Result<Vec<SessionEventRecord>, AstrError> {
        self.inner.load_history(session_id).await
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use chrono::Utc;

    use super::{SessionTruthRuntime, SessionTruthSurface};

    #[derive(Clone)]
    struct StubSessionTruth;

    #[async_trait]
    impl SessionTruthRuntime for StubSessionTruth {
        async fn create_session(
            &self,
            working_dir: &std::path::Path,
        ) -> std::result::Result<astrcode_core::SessionMeta, astrcode_core::AstrError> {
            Ok(astrcode_core::SessionMeta {
                session_id: "session-1".to_string(),
                working_dir: working_dir.to_string_lossy().to_string(),
                display_name: "demo".to_string(),
                title: "title".to_string(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                parent_session_id: None,
                parent_storage_seq: None,
                phase: astrcode_core::Phase::Idle,
            })
        }

        async fn list_sessions(
            &self,
        ) -> std::result::Result<Vec<astrcode_core::SessionMeta>, astrcode_core::AstrError>
        {
            Ok(Vec::new())
        }

        async fn load_history(
            &self,
            _session_id: &str,
        ) -> std::result::Result<Vec<astrcode_core::SessionEventRecord>, astrcode_core::AstrError>
        {
            Ok(Vec::new())
        }
    }

    #[test]
    fn runtime_session_boundary_metadata_declares_owner_and_surface() {
        let manifest = include_str!("../Cargo.toml");

        assert!(manifest.contains("owner = \"session-truth\""));
        assert!(manifest.contains(
            "public_surface = [\"session-state\", \"history\", \"replay\", \"catalog\"]"
        ));
        assert!(manifest.contains("target_depends_on = [\"astrcode-core\"]"));
    }

    #[test]
    fn runtime_session_boundary_metadata_tracks_legacy_dependency_removal_plan() {
        let manifest = include_str!("../Cargo.toml");

        assert!(manifest.contains("legacy_depends_on = ["));
        assert!(manifest.contains("\"astrcode-runtime-agent-control\""));
        assert!(manifest.contains("\"astrcode-runtime-agent-loop\""));
        assert!(
            manifest.contains(
                "migration_note = \"将剥离 turn orchestration 并移除 legacy_depends_on\""
            )
        );
    }

    #[tokio::test]
    async fn session_truth_surface_delegates_core_boundary_calls() {
        let surface = SessionTruthSurface::new(StubSessionTruth);

        let created = astrcode_core::SessionTruthBoundary::create_session(
            &surface,
            std::path::Path::new("."),
        )
        .await
        .expect("session should be created");

        assert_eq!(created.session_id, "session-1");
        assert!(
            astrcode_core::SessionTruthBoundary::list_sessions(&surface)
                .await
                .expect("list should succeed")
                .is_empty()
        );
        assert!(
            astrcode_core::SessionTruthBoundary::load_history(&surface, "session-1")
                .await
                .expect("history should succeed")
                .is_empty()
        );
    }
}
