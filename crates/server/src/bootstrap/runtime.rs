//! # 服务器运行时组合根
//!
//! 由 server 显式组装 adapter、kernel、session-runtime、application。
//! 所有 provider 和 gateway 在此唯一位置接线，handler 只依赖 `App`。

use std::sync::Arc;

use astrcode_adapter_storage::core_port::FsEventStore;
use astrcode_application::{App, AppGovernance};
use astrcode_core::{EventStore, LlmProvider, PromptProvider, ResourceProvider, Result};
use astrcode_kernel::{CapabilityRouter, Kernel, KernelBuilder};
use astrcode_session_runtime::SessionRuntime;

use super::{
    capabilities::{
        CapabilitySurfaceSync, build_builtin_capability_invokers, build_server_capability_router,
    },
    governance::build_app_governance,
    mcp::{bootstrap_mcp_manager, build_mcp_service},
    providers::{
        build_config_service, build_llm_provider, build_prompt_provider, build_resource_provider,
    },
};

/// 服务器运行时：组合根输出。
pub struct ServerRuntime {
    pub app: Arc<App>,
    pub governance: Arc<AppGovernance>,
}

/// 构建服务器运行时组合根。
pub async fn bootstrap_server_runtime() -> Result<ServerRuntime> {
    let config_service = build_config_service()?;
    let working_dir = std::env::current_dir().map_err(|error| {
        astrcode_core::AstrError::io("failed to resolve server working directory", error)
    })?;
    let mcp_manager = bootstrap_mcp_manager(config_service.clone(), working_dir.as_path()).await?;

    let builtin_invokers = build_builtin_capability_invokers()?;
    let mut all_invokers = builtin_invokers.clone();
    all_invokers.extend(mcp_manager.current_surface().await.capability_invokers);
    let capabilities = build_server_capability_router(all_invokers)?;

    let kernel = Arc::new(build_kernel(
        capabilities,
        build_llm_provider(config_service.clone(), working_dir.clone()),
        build_prompt_provider(),
        build_resource_provider(mcp_manager.clone()),
    )?);
    let capability_sync = CapabilitySurfaceSync::new(kernel.clone(), builtin_invokers);

    let event_store: Arc<dyn EventStore> = Arc::new(FsEventStore::new());
    let session_runtime = Arc::new(SessionRuntime::new(kernel.clone(), event_store));
    let mcp_service = build_mcp_service(
        config_service.clone(),
        working_dir,
        mcp_manager,
        capability_sync,
    );

    let app = Arc::new(App::new(
        kernel.clone(),
        session_runtime.clone(),
        config_service,
        mcp_service,
    ));
    let governance = build_app_governance(session_runtime, kernel.clone());

    Ok(ServerRuntime { app, governance })
}

fn build_kernel(
    capabilities: CapabilityRouter,
    llm_provider: Arc<dyn LlmProvider>,
    prompt_provider: Arc<dyn PromptProvider>,
    resource_provider: Arc<dyn ResourceProvider>,
) -> Result<Kernel> {
    KernelBuilder::default()
        .with_capabilities(capabilities)
        .with_llm_provider(llm_provider)
        .with_prompt_provider(prompt_provider)
        .with_resource_provider(resource_provider)
        .build()
        .map_err(|error| astrcode_core::AstrError::Internal(error.to_string()))
}
