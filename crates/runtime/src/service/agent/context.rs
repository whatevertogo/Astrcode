//! agent 编排边界的延迟执行器桥。
//!
//! builtin router 在 `RuntimeService` 创建前就要注册协作工具，
//! 因此这里先占位，等 runtime 创建完成后再绑定真实 service。

use std::sync::{Arc, RwLock as StdRwLock, Weak};

use astrcode_core::{
    AstrError, CloseAgentParams, CollaborationResult, DeliverToParentParams, ObserveParams, Result,
    ResumeAgentParams, SendAgentParams, ToolContext, WaitAgentParams,
};
use astrcode_runtime_agent_tool::CollaborationExecutor;
use async_trait::async_trait;

use crate::service::{RuntimeService, ServiceError};

/// ServiceError 到 AstrError 的转换。
pub(crate) fn service_error_to_astr(error: ServiceError) -> AstrError {
    match error {
        ServiceError::NotFound(message)
        | ServiceError::Conflict(message)
        | ServiceError::InvalidInput(message) => AstrError::Validation(message),
        ServiceError::Internal(error) => error,
    }
}

/// 协作工具的延迟执行器桥。
#[derive(Default)]
pub(crate) struct DeferredCollaborationExecutor {
    runtime: StdRwLock<Option<Weak<RuntimeService>>>,
}

impl DeferredCollaborationExecutor {
    pub(crate) fn bind(&self, runtime: &Arc<RuntimeService>) {
        let mut guard = self
            .runtime
            .write()
            .expect("collaboration executor binding lock should not be poisoned");
        *guard = Some(Arc::downgrade(runtime));
    }

    fn runtime(&self) -> Result<Arc<RuntimeService>> {
        let guard = self
            .runtime
            .read()
            .expect("collaboration executor binding lock should not be poisoned");
        let Some(runtime) = guard.as_ref().and_then(Weak::upgrade) else {
            return Err(AstrError::Internal(
                "collaboration executor is not bound to runtime service yet".to_string(),
            ));
        };
        Ok(runtime)
    }
}

#[async_trait]
impl CollaborationExecutor for DeferredCollaborationExecutor {
    async fn send(
        &self,
        params: SendAgentParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult> {
        let runtime = self.runtime()?;
        runtime
            .agent()
            .send_to_child(params, ctx)
            .await
            .map_err(service_error_to_astr)
    }

    async fn wait(
        &self,
        params: WaitAgentParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult> {
        let runtime = self.runtime()?;
        runtime
            .agent()
            .wait_for_child(params, ctx)
            .await
            .map_err(service_error_to_astr)
    }

    async fn close(
        &self,
        params: CloseAgentParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult> {
        let runtime = self.runtime()?;
        runtime
            .agent()
            .close_child(params, ctx)
            .await
            .map_err(service_error_to_astr)
    }

    async fn resume(
        &self,
        params: ResumeAgentParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult> {
        let runtime = self.runtime()?;
        runtime
            .agent()
            .resume_child(params, ctx)
            .await
            .map_err(service_error_to_astr)
    }

    async fn deliver(
        &self,
        params: DeliverToParentParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult> {
        let runtime = self.runtime()?;
        runtime
            .agent()
            .deliver_to_parent(params, ctx)
            .await
            .map_err(service_error_to_astr)
    }

    async fn observe(
        &self,
        params: ObserveParams,
        ctx: &ToolContext,
    ) -> Result<CollaborationResult> {
        let runtime = self.runtime()?;
        runtime
            .agent()
            .observe_child(params, ctx)
            .await
            .map_err(service_error_to_astr)
    }
}
