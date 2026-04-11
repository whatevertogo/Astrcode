//! 执行上下文：bootstrap 阶段的延迟执行器桥。
//!
//! builtin router 在 `RuntimeService` 创建前就要注册 `spawn`，
//! 因此这里先占位，等 service 创建完成后再绑定真实 runtime。

use std::sync::{Arc, RwLock as StdRwLock, Weak};

use astrcode_core::{AstrError, Result, SpawnAgentParams, SubRunResult, ToolContext};
use astrcode_runtime_agent_tool::SubAgentExecutor;
use async_trait::async_trait;

use crate::service::{RuntimeService, service_error_to_astr};

/// bootstrap 阶段使用的延迟执行器桥。
///
/// builtin router 在 `RuntimeService` 创建前就要注册 `spawn`，因此这里先占位，
/// 等 service 创建完成后再绑定真实 runtime。
#[derive(Default)]
pub(crate) struct DeferredSubAgentExecutor {
    runtime: StdRwLock<Option<Weak<RuntimeService>>>,
}

impl DeferredSubAgentExecutor {
    pub(crate) fn bind(&self, runtime: &Arc<RuntimeService>) {
        let mut guard = self
            .runtime
            .write()
            .expect("sub-agent executor binding lock should not be poisoned");
        *guard = Some(Arc::downgrade(runtime));
    }

    fn runtime(&self) -> Result<Arc<RuntimeService>> {
        let guard = self
            .runtime
            .read()
            .expect("sub-agent executor binding lock should not be poisoned");
        let Some(runtime) = guard.as_ref().and_then(Weak::upgrade) else {
            return Err(AstrError::Internal(
                "spawn executor is not bound to runtime service yet".to_string(),
            ));
        };
        Ok(runtime)
    }
}

#[async_trait]
impl SubAgentExecutor for DeferredSubAgentExecutor {
    async fn launch(&self, params: SpawnAgentParams, ctx: &ToolContext) -> Result<SubRunResult> {
        let runtime = self.runtime()?;
        runtime
            .execution()
            .launch_subagent(params, ctx)
            .await
            .map_err(service_error_to_astr)
    }
}
