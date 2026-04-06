use crate::service::{PromptAccepted, RuntimeService, ServiceResult};

impl RuntimeService {
    pub async fn submit_prompt(
        &self,
        session_id: &str,
        text: String,
    ) -> ServiceResult<PromptAccepted> {
        self.execution_service()
            .submit_prompt(session_id, text)
            .await
    }

    pub async fn interrupt(&self, session_id: &str) -> ServiceResult<()> {
        self.execution_service().interrupt(session_id).await
    }
}
