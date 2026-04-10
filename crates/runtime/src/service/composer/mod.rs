mod service;

use std::sync::Arc;

use crate::service::{ComposerOption, RuntimeService, ServiceResult};

/// `runtime-composer` 的唯一 surface handle。
#[derive(Clone)]
pub struct ComposerServiceHandle {
    runtime: Arc<RuntimeService>,
}

impl ComposerServiceHandle {
    pub(super) fn new(runtime: Arc<RuntimeService>) -> Self {
        Self { runtime }
    }

    fn service(&self) -> service::ComposerService<'_> {
        service::ComposerService::new(self.runtime.as_ref())
    }

    pub async fn list_composer_options(
        &self,
        session_id: &str,
        request: crate::service::ComposerOptionsRequest,
    ) -> ServiceResult<Vec<ComposerOption>> {
        self.service()
            .list_composer_options(session_id, request)
            .await
    }
}
