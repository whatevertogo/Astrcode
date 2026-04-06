use std::path::PathBuf;

use astrcode_core::SessionMeta;

use crate::service::{RuntimeService, ServiceResult};

impl RuntimeService {
    pub async fn list_sessions_with_meta(&self) -> ServiceResult<Vec<SessionMeta>> {
        self.session_service().list_sessions_with_meta().await
    }

    pub async fn create_session(
        &self,
        working_dir: impl Into<PathBuf>,
    ) -> ServiceResult<SessionMeta> {
        self.session_service().create_session(working_dir).await
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::project::project_dir_name;

    use super::*;
    use crate::test_support::{TestEnvGuard, empty_capabilities};

    #[tokio::test]
    async fn create_session_persists_into_project_bucket_directory() {
        let guard = TestEnvGuard::new();
        let service = RuntimeService::from_capabilities(empty_capabilities()).unwrap();
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");

        let meta = service
            .create_session(temp_dir.path())
            .await
            .expect("session should be created");

        let projects_root = guard.home_dir().join(".astrcode").join("projects");
        assert!(
            !guard
                .home_dir()
                .join(".astrcode")
                .join("sessions")
                .join(format!("session-{}.jsonl", meta.session_id))
                .exists(),
            "new layout should avoid writing fresh sessions back into the legacy flat root"
        );

        let bucket_dir = projects_root
            .join(project_dir_name(temp_dir.path()))
            .join("sessions");
        let session_dir = bucket_dir.join(&meta.session_id);
        assert!(
            session_dir
                .join(format!("session-{}.jsonl", meta.session_id))
                .exists(),
            "session file should be nested under a per-session directory inside the project bucket"
        );
    }
}
