//! # 运行时治理 (Runtime Governance)
//!
//! 提供运行时的治理和可观测性能力，包括：
//! - 运行时快照（会话状态、插件健康、能力列表、指标）
//! - 插件热重载（无需重启服务）
//! - 插件健康探针刷新
//!
//! ## 重载约束
//!
//! 重载运行时能力时，不允许有任何正在运行的会话。这是为了保证
//! 会话状态的一致性——如果会话正在执行 Turn，重载能力会导致
//! 工具调用中断或状态不一致。
//!
//! ## 重载流程
//!
//! 1. 获取重载锁（防止并发重载）
//! 2. 检查是否有运行中的会话（有则拒绝）
//! 3. 重新发现插件并组装能力面
//! 4. 替换 RuntimeService 的能力路由
//! 5. 替换 RuntimeCoordinator 的能力面
//! 6. 关闭旧的托管组件
//! 7. 返回新的快照

use std::{path::PathBuf, sync::Arc};

use astrcode_core::{
    PluginHealth, PluginManifest, RuntimeCoordinator, format_local_rfc3339, plugin::PluginEntry,
};
use astrcode_protocol::capability::CapabilityDescriptor;
use chrono::{DateTime, Utc};
use tokio::sync::{Mutex, RwLock};

use crate::{
    RuntimeObservabilitySnapshot, RuntimeService, ServiceError,
    plugin_discovery::{configured_plugin_paths, discover_plugin_manifests_in},
    runtime_surface_assembler::{
        ActivePluginRuntime, PluginInitializer, SupervisorPluginInitializer,
        assemble_runtime_surface,
    },
};

#[derive(Debug, Clone)]
pub struct RuntimeGovernanceSnapshot {
    pub runtime_name: String,
    pub runtime_kind: String,
    pub loaded_session_count: usize,
    pub running_session_ids: Vec<String>,
    pub plugin_search_paths: Vec<PathBuf>,
    pub metrics: RuntimeObservabilitySnapshot,
    pub capabilities: Vec<CapabilityDescriptor>,
    pub plugins: Vec<PluginEntry>,
}

/// 运行时重载操作的结果。
///
/// 包含重载后的新快照和重载完成的时间戳，
/// 供调用方确认重载是否成功以及何时生效。
#[derive(Debug, Clone)]
pub struct RuntimeReloadResult {
    /// 重载后的运行时快照
    pub snapshot: RuntimeGovernanceSnapshot,
    /// 重载完成的时间
    pub reloaded_at: DateTime<Utc>,
}

pub struct RuntimeGovernance {
    service: Arc<RuntimeService>,
    coordinator: Arc<RuntimeCoordinator>,
    active_plugins: RwLock<Vec<ActivePluginRuntime>>,
    reload_lock: Mutex<()>,
}

impl RuntimeGovernance {
    pub fn from_runtime(
        service: Arc<RuntimeService>,
        coordinator: Arc<RuntimeCoordinator>,
    ) -> Self {
        Self::with_active_plugins(service, coordinator, Vec::new())
    }

    pub(crate) fn with_active_plugins(
        service: Arc<RuntimeService>,
        coordinator: Arc<RuntimeCoordinator>,
        active_plugins: Vec<ActivePluginRuntime>,
    ) -> Self {
        Self {
            service,
            coordinator,
            active_plugins: RwLock::new(active_plugins),
            reload_lock: Mutex::new(()),
        }
    }

    /// 更新活跃插件列表（用于后台加载完成后同步状态）。
    ///
    /// 此方法仅更新内部活跃插件列表，不触发生命周期管理。
    /// 用于启动时后台加载插件完成后，将插件信息同步到 governance。
    pub(crate) async fn update_active_plugins(&self, active_plugins: Vec<ActivePluginRuntime>) {
        let mut guard = self.active_plugins.write().await;
        *guard = active_plugins;
    }

    pub async fn snapshot(&self) -> RuntimeGovernanceSnapshot {
        self.refresh_plugin_health().await;
        self.snapshot_with_paths(configured_plugin_paths()).await
    }

    pub async fn reload(&self) -> Result<RuntimeReloadResult, ServiceError> {
        let search_paths = configured_plugin_paths();
        let manifests =
            discover_plugin_manifests_in(&search_paths).map_err(ServiceError::Internal)?;
        let initializer = SupervisorPluginInitializer::new(search_paths.clone());
        self.reload_from_manifests(manifests, &initializer, search_paths)
            .await
    }

    pub(crate) async fn reload_from_manifests<I>(
        &self,
        manifests: Vec<PluginManifest>,
        initializer: &I,
        plugin_search_paths: Vec<PathBuf>,
    ) -> Result<RuntimeReloadResult, ServiceError>
    where
        I: PluginInitializer,
    {
        let _guard = self.reload_lock.lock().await;
        let running_session_ids = self.service.running_session_ids();
        if !running_session_ids.is_empty() {
            return Err(ServiceError::Conflict(format!(
                "cannot reload runtime capabilities while sessions are running: {}",
                running_session_ids.join(", ")
            )));
        }

        let assembled = assemble_runtime_surface(
            manifests,
            initializer,
            self.coordinator.plugin_registry(),
            astrcode_runtime_skill_loader::load_builtin_skills(),
            Arc::new(self.service.execution()),
            self.service.agent().collaboration_executor(),
        )
        .await
        .map_err(ServiceError::Internal)?;
        let capability_surface = assembled.router.descriptors();
        self.service
            .loop_surface()
            .replace_surface(
                assembled.router,
                assembled.prompt_declarations,
                assembled.skill_catalog,
                assembled.hook_handlers,
            )
            .await?;
        let previous_active_plugins = {
            let mut guard = self.active_plugins.write().await;
            std::mem::replace(&mut *guard, assembled.active_plugins)
        };
        let retired_components = self.coordinator.replace_runtime_surface(
            assembled.plugin_entries,
            capability_surface,
            assembled.managed_components,
        );
        for component in retired_components {
            if let Err(error) = component.shutdown_component().await {
                log::warn!(
                    "failed to shut down retired managed component '{}' after reload: {}",
                    component.component_name(),
                    error
                );
            }
        }

        drop(previous_active_plugins);

        Ok(RuntimeReloadResult {
            snapshot: self.snapshot_with_paths(plugin_search_paths).await,
            reloaded_at: Utc::now(),
        })
    }

    async fn snapshot_with_paths(
        &self,
        plugin_search_paths: Vec<PathBuf>,
    ) -> RuntimeGovernanceSnapshot {
        let runtime = self.coordinator.runtime();
        RuntimeGovernanceSnapshot {
            runtime_name: runtime.runtime_name().to_string(),
            runtime_kind: runtime.runtime_kind().to_string(),
            loaded_session_count: self.service.loaded_session_count(),
            running_session_ids: self.service.running_session_ids(),
            plugin_search_paths,
            metrics: self.service.observability().snapshot(),
            capabilities: self.coordinator.capabilities(),
            plugins: self.coordinator.plugin_registry().snapshot(),
        }
    }

    async fn refresh_plugin_health(&self) {
        let active_plugins = self.active_plugins.read().await.clone();
        for active_plugin in active_plugins {
            // 检查时间会直接展示在运行时状态里，统一输出为本地时区更符合用户排障直觉。
            let checked_at = format_local_rfc3339(Utc::now());
            match active_plugin.component.health_report().await {
                Ok(report) => match report.health {
                    PluginHealth::Healthy => {
                        self.coordinator.plugin_registry().record_health_probe(
                            &active_plugin.name,
                            PluginHealth::Healthy,
                            None,
                            checked_at,
                        );
                    },
                    PluginHealth::Unavailable | PluginHealth::Degraded | PluginHealth::Unknown => {
                        let message = report
                            .message
                            .unwrap_or_else(|| "plugin supervisor unavailable".to_string());
                        self.coordinator.plugin_registry().record_health_probe(
                            &active_plugin.name,
                            PluginHealth::Unavailable,
                            Some(message),
                            checked_at,
                        );
                    },
                },
                Err(error) => {
                    self.coordinator.plugin_registry().record_health_probe(
                        &active_plugin.name,
                        PluginHealth::Unavailable,
                        Some(error.to_string()),
                        checked_at,
                    );
                },
            }
        }
    }
}
