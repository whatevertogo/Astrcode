#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use astrcode_core::project::project_dir_name;

    use crate::{
        service::RuntimeService,
        test_support::{TestEnvGuard, empty_capabilities},
    };

    #[tokio::test]
    async fn create_session_persists_into_project_bucket_directory() {
        let guard = TestEnvGuard::new();
        let service = Arc::new(RuntimeService::from_capabilities(empty_capabilities()).unwrap());
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");

        let meta = service
            .sessions()
            .create(temp_dir.path())
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
