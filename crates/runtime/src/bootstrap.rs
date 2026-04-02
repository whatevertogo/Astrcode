//! # 运行时引导 (Runtime Bootstrap)
//!
//! 负责初始化整个运行时系统，包括：
//! 1. 发现并加载插件清单
//! 2. 组装运行时能力面（内置工具 + 插件）
//! 3. 创建 `RuntimeService`（核心门面）
//! 4. 创建 `RuntimeCoordinator`（协调器）
//! 5. 创建 `RuntimeGovernance`（治理层，支持热重载）
//!
//! ## 引导时序
//!
//! ```text
//! 插件发现 → 能力面组装 → RuntimeService → RuntimeCoordinator → RuntimeGovernance
//! ```
//!
//! 每个步骤失败都会导致引导终止，返回 `AstrError`。
//! 引导完成后，调用方获得 `RuntimeBootstrap` 结构体，
//! 包含 service、coordinator 和 governance 三个核心组件。

use std::sync::Arc;

use astrcode_core::{AstrError, PluginManifest, PluginRegistry, RuntimeCoordinator, RuntimeHandle};

use crate::plugin_discovery::{configured_plugin_paths, discover_plugin_manifests_in};
use crate::runtime_governance::RuntimeGovernance;
use crate::runtime_surface_assembler::{
    assemble_runtime_surface, PluginInitializer, SupervisorPluginInitializer,
};
use crate::{RuntimeService, ServiceError};

/// 运行时引导完成后的结果容器。
///
/// 包含三个核心组件：
/// - `service`: `RuntimeService` 门面，处理所有会话和 Turn 操作
/// - `coordinator`: `RuntimeCoordinator`，管理插件生命周期和托管组件
/// - `governance`: `RuntimeGovernance`，提供治理和可观测性能力（如热重载）
pub struct RuntimeBootstrap {
    /// 运行时服务门面，所有会话/工具/Turn 操作的入口
    pub service: Arc<RuntimeService>,
    /// 运行时协调器，管理插件注册和托管组件
    pub coordinator: Arc<RuntimeCoordinator>,
    /// 运行时治理层，支持快照、热重载等治理能力
    pub governance: Arc<RuntimeGovernance>,
}

/// 引导运行时系统。
///
/// 按以下顺序初始化：
/// 1. 从环境变量发现插件搜索路径
/// 2. 扫描并加载插件清单
/// 3. 组装运行时能力面（内置工具 + 插件）
/// 4. 创建 `RuntimeService`、`RuntimeCoordinator`、`RuntimeGovernance`
///
/// 任何步骤失败都会终止引导并返回 `AstrError`。
pub async fn bootstrap_runtime() -> std::result::Result<RuntimeBootstrap, AstrError> {
    let search_paths = configured_plugin_paths();
    let manifests = discover_plugin_manifests_in(&search_paths)?;
    let initializer = SupervisorPluginInitializer::new(search_paths);
    bootstrap_runtime_from_manifests(manifests, &initializer).await
}

pub(crate) async fn bootstrap_runtime_from_manifests<I>(
    manifests: Vec<PluginManifest>,
    initializer: &I,
) -> std::result::Result<RuntimeBootstrap, AstrError>
where
    I: PluginInitializer,
{
    let plugin_registry = Arc::new(PluginRegistry::default());
    let builtin_skills = crate::prompt::load_builtin_skills();
    let assembled = assemble_runtime_surface(
        manifests,
        initializer,
        Arc::clone(&plugin_registry),
        builtin_skills.clone(),
    )
    .await?;
    let capability_surface = assembled.router.descriptors();
    plugin_registry.replace_snapshot(assembled.plugin_entries);
    let service = Arc::new(
        RuntimeService::from_capabilities_with_prompt_inputs(
            assembled.router,
            assembled.prompt_declarations,
            builtin_skills,
        )
        .map_err(service_error_to_astr)?,
    );
    let runtime: Arc<dyn RuntimeHandle> = service.clone();
    let coordinator = Arc::new(
        RuntimeCoordinator::new(runtime, plugin_registry, capability_surface)
            .with_managed_components(assembled.managed_components),
    );
    let governance = Arc::new(RuntimeGovernance::with_active_plugins(
        Arc::clone(&service),
        Arc::clone(&coordinator),
        assembled.active_plugins,
    ));

    Ok(RuntimeBootstrap {
        service,
        coordinator,
        governance,
    })
}

fn service_error_to_astr(error: ServiceError) -> AstrError {
    match error {
        ServiceError::NotFound(message)
        | ServiceError::Conflict(message)
        | ServiceError::InvalidInput(message) => AstrError::Validation(message),
        ServiceError::Internal(error) => AstrError::Internal(error.to_string()),
    }
}
