use astrcode_core::DeleteProjectResult;

use crate::service::{RuntimeService, ServiceResult};

impl RuntimeService {
    pub async fn delete_session(&self, session_id: &str) -> ServiceResult<()> {
        self.session_service().delete_session(session_id).await
    }

    pub async fn delete_project(&self, working_dir: &str) -> ServiceResult<DeleteProjectResult> {
        self.session_service().delete_project(working_dir).await
    }
}
