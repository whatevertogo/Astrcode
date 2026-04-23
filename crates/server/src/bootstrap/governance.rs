//! # 治理装配
//!
//! 负责把底层 `RuntimeCoordinator` 适配成应用层治理端口，
//! 并为治理入口接入真实 reload/observability 组合根。

use std::{collections::HashSet, path::PathBuf, sync::Arc};

use astrcode_adapter_mcp::{
    config::McpServerConfig,
    manager::{McpConnectionManager, McpReloadSnapshot},
};
use astrcode_adapter_skills::{LayeredSkillCatalog, load_builtin_skills};
use astrcode_application::{
    AppGovernance, ApplicationError, ModeCatalog, RuntimeGovernancePort, RuntimeGovernanceSnapshot,
    RuntimeObservabilityCollector, RuntimeReloader, SessionInfoProvider, config::ConfigService,
    lifecycle::TaskRegistry, mode::ModeCatalogSnapshot,
};
use astrcode_core::{CapabilityInvoker, SkillSpec, plugin::PluginEntry};
use astrcode_plugin::Supervisor;
use async_trait::async_trait;

use super::{
    capabilities::CapabilitySurfaceSync,
    deps::{
        core::{AstrError, ManagedRuntimeComponent, RuntimeHandle},
        session_runtime::SessionRuntime,
    },
    mcp::load_declared_configs,
    plugins::bootstrap_plugins_with_skill_root,
    runtime_coordinator::RuntimeCoordinator,
};

pub(crate) struct GovernanceBuildInput {
    pub session_runtime: Arc<SessionRuntime>,
    pub config_service: Arc<ConfigService>,
    pub coordinator: Arc<RuntimeCoordinator>,
    pub task_registry: Arc<TaskRegistry>,
    pub observability: Arc<RuntimeObservabilityCollector>,
    pub mcp_manager: Arc<McpConnectionManager>,
    pub capability_sync: CapabilitySurfaceSync,
    pub skill_catalog: Arc<LayeredSkillCatalog>,
    pub plugin_search_paths: Vec<PathBuf>,
    pub plugin_skill_root: PathBuf,
    pub plugin_supervisors: Vec<Arc<Supervisor>>,
    pub working_dir: PathBuf,
    pub mode_catalog: Option<Arc<ModeCatalog>>,
}

pub(crate) fn build_app_governance(input: GovernanceBuildInput) -> Arc<AppGovernance> {
    let runtime_port = Arc::new(CoordinatorGovernancePort {
        coordinator: Arc::clone(&input.coordinator),
    });
    let sessions = Arc::new(SessionRuntimeInfo {
        session_runtime: Arc::clone(&input.session_runtime),
    });
    let reloader: Arc<dyn RuntimeReloader> = Arc::new(ServerRuntimeReloader {
        config_service: Arc::clone(&input.config_service),
        coordinator: Arc::clone(&input.coordinator),
        mcp_manager: Arc::clone(&input.mcp_manager),
        capability_sync: input.capability_sync.clone(),
        skill_catalog: Arc::clone(&input.skill_catalog),
        plugin_search_paths: input.plugin_search_paths.clone(),
        plugin_skill_root: input.plugin_skill_root.clone(),
        working_dir: input.working_dir.clone(),
        mode_catalog: input.mode_catalog,
    });
    let managed_components: Vec<Arc<dyn ManagedRuntimeComponent>> = input
        .plugin_supervisors
        .into_iter()
        .map(|supervisor| supervisor as Arc<dyn ManagedRuntimeComponent>)
        .collect();
    input.coordinator.replace_runtime_surface(
        input.coordinator.plugin_registry().snapshot(),
        input.capability_sync.current_capabilities(),
        managed_components,
    );

    Arc::new(
        AppGovernance::new(
            runtime_port,
            input.task_registry,
            input.observability,
            sessions,
        )
        .with_reloader(reloader),
    )
}

#[derive(Debug)]
struct CoordinatorGovernancePort {
    coordinator: Arc<RuntimeCoordinator>,
}

impl RuntimeGovernancePort for CoordinatorGovernancePort {
    fn snapshot(&self) -> RuntimeGovernanceSnapshot {
        let runtime = self.coordinator.runtime();
        RuntimeGovernanceSnapshot {
            runtime_name: runtime.runtime_name().to_string(),
            runtime_kind: runtime.runtime_kind().to_string(),
            capabilities: self.coordinator.capabilities(),
            plugins: self.coordinator.plugin_registry().snapshot(),
        }
    }

    fn shutdown(
        &self,
        timeout_secs: u64,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = std::result::Result<(), ApplicationError>> + Send + '_,
        >,
    > {
        Box::pin(async move {
            self.coordinator
                .shutdown(timeout_secs)
                .await
                .map_err(|error| ApplicationError::Internal(error.to_string()))
        })
    }
}

#[derive(Debug)]
pub(crate) struct AppRuntimeHandle;

#[async_trait]
impl RuntimeHandle for AppRuntimeHandle {
    fn runtime_name(&self) -> &'static str {
        "astrcode-application"
    }

    fn runtime_kind(&self) -> &'static str {
        "application"
    }

    async fn shutdown(&self, _timeout_secs: u64) -> std::result::Result<(), AstrError> {
        Ok(())
    }
}

struct SessionRuntimeInfo {
    session_runtime: Arc<SessionRuntime>,
}

impl SessionInfoProvider for SessionRuntimeInfo {
    fn loaded_session_count(&self) -> usize {
        self.session_runtime.list_sessions().len()
    }

    fn running_session_ids(&self) -> Vec<String> {
        self.session_runtime
            .list_running_sessions()
            .into_iter()
            .map(|id| id.to_string())
            .collect()
    }
}

impl std::fmt::Debug for SessionRuntimeInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionRuntimeInfo").finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct ServerRuntimeReloader {
    config_service: Arc<ConfigService>,
    coordinator: Arc<RuntimeCoordinator>,
    mcp_manager: Arc<McpConnectionManager>,
    capability_sync: CapabilitySurfaceSync,
    skill_catalog: Arc<LayeredSkillCatalog>,
    plugin_search_paths: Vec<PathBuf>,
    plugin_skill_root: PathBuf,
    working_dir: PathBuf,
    mode_catalog: Option<Arc<ModeCatalog>>,
}

impl std::fmt::Debug for ServerRuntimeReloader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerRuntimeReloader")
            .field("plugin_search_paths", &self.plugin_search_paths)
            .field("plugin_skill_root", &self.plugin_skill_root)
            .finish_non_exhaustive()
    }
}

struct PreparedGovernanceReload {
    search_paths: Vec<PathBuf>,
    mcp_configs: Vec<McpServerConfig>,
    mode_snapshot: Option<ModeCatalogSnapshot>,
    base_skills: Vec<SkillSpec>,
    plugin_invokers: Vec<Arc<dyn CapabilityInvoker>>,
    plugin_entries: Vec<PluginEntry>,
    managed_components: Vec<Arc<dyn ManagedRuntimeComponent>>,
}

struct GovernanceReloadRollback {
    mcp_snapshot: McpReloadSnapshot,
    plugin_invokers: Vec<Arc<dyn CapabilityInvoker>>,
}

impl GovernanceReloadRollback {
    async fn capture(
        mcp_manager: &McpConnectionManager,
        capability_sync: &CapabilitySurfaceSync,
    ) -> Self {
        let mcp_snapshot = mcp_manager.capture_reload_snapshot().await;
        let mcp_surface = mcp_manager.current_surface().await;
        let mcp_capability_names = mcp_surface
            .capability_invokers
            .into_iter()
            .map(|invoker| invoker.capability_spec().name.to_string())
            .collect::<HashSet<_>>();
        let plugin_invokers = capability_sync
            .current_external_invokers()
            .into_iter()
            .filter(|invoker| {
                !mcp_capability_names.contains(invoker.capability_spec().name.as_str())
            })
            .collect();
        Self {
            mcp_snapshot,
            plugin_invokers,
        }
    }

    async fn restore(
        self,
        mcp_manager: &McpConnectionManager,
        capability_sync: &CapabilitySurfaceSync,
    ) -> Result<(), ApplicationError> {
        let mut external_invokers = mcp_manager
            .restore_reload_snapshot(&self.mcp_snapshot)
            .await
            .map_err(|error| ApplicationError::Internal(error.to_string()))?;
        external_invokers.extend(self.plugin_invokers);
        capability_sync
            .apply_external_invokers(external_invokers)
            .map_err(|error| ApplicationError::Internal(error.to_string()))
    }
}

impl ServerRuntimeReloader {
    async fn prepare_reload_candidate(&self) -> Result<PreparedGovernanceReload, ApplicationError> {
        let mcp_configs =
            load_declared_configs(&self.config_service, self.working_dir.as_path()).await?;
        let plugin_bootstrap = bootstrap_plugins_with_skill_root(
            self.plugin_search_paths.clone(),
            self.plugin_skill_root.clone(),
        )
        .await;
        let mode_snapshot = match &self.mode_catalog {
            Some(mode_catalog) => Some(
                mode_catalog
                    .preview_plugin_modes(plugin_bootstrap.modes.clone())
                    .map_err(ApplicationError::from)?,
            ),
            None => None,
        };

        let mut base_skills = load_builtin_skills();
        base_skills.extend(plugin_bootstrap.skills.clone());
        let managed_components: Vec<Arc<dyn ManagedRuntimeComponent>> = plugin_bootstrap
            .supervisors
            .iter()
            .cloned()
            .map(|supervisor| supervisor as Arc<dyn ManagedRuntimeComponent>)
            .collect();

        Ok(PreparedGovernanceReload {
            search_paths: plugin_bootstrap.search_paths,
            mcp_configs,
            mode_snapshot,
            base_skills,
            plugin_invokers: plugin_bootstrap.invokers,
            plugin_entries: plugin_bootstrap.registry.snapshot(),
            managed_components,
        })
    }

    async fn shutdown_replaced_components(
        &self,
        previous_components: Vec<Arc<dyn ManagedRuntimeComponent>>,
    ) {
        for component in previous_components {
            if let Err(error) = component.shutdown_component().await {
                log::warn!(
                    "failed to shut down replaced managed component '{}': {}",
                    component.component_name(),
                    error
                );
            }
        }
    }
}

impl RuntimeReloader for ServerRuntimeReloader {
    fn reload(
        &self,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<PathBuf>, ApplicationError>> + Send + '_>,
    > {
        Box::pin(async move {
            self.config_service.reload_from_disk().await?;
            let candidate = self.prepare_reload_candidate().await?;
            let rollback =
                GovernanceReloadRollback::capture(&self.mcp_manager, &self.capability_sync).await;

            let mcp_invokers = self
                .mcp_manager
                .reload_config(candidate.mcp_configs)
                .await
                .map_err(|error| ApplicationError::Internal(error.to_string()))?;
            let mut external_invokers = mcp_invokers;
            external_invokers.extend(candidate.plugin_invokers.clone());

            if let Err(error) = self
                .capability_sync
                .apply_external_invokers(external_invokers)
            {
                let error = ApplicationError::Internal(error.to_string());
                log::error!(
                    "governance reload failed while applying candidate capability surface: {error}"
                );
                if let Err(rollback_error) = rollback
                    .restore(&self.mcp_manager, &self.capability_sync)
                    .await
                {
                    return Err(ApplicationError::Internal(format!(
                        "governance reload failed: {}; rollback failed: {}",
                        error, rollback_error
                    )));
                }
                log::warn!(
                    "governance reload rolled back to previous external capability snapshot"
                );
                return Err(error);
            }

            self.skill_catalog
                .replace_base_skills(candidate.base_skills);
            if let (Some(mode_catalog), Some(mode_snapshot)) =
                (&self.mode_catalog, candidate.mode_snapshot)
            {
                mode_catalog.replace_snapshot(mode_snapshot);
            }
            let previous_components = self.coordinator.replace_runtime_surface(
                candidate.plugin_entries,
                self.capability_sync.current_capabilities(),
                candidate.managed_components,
            );
            self.shutdown_replaced_components(previous_components).await;
            log::info!(
                "governance reload committed: plugin_search_paths={}, base_skills={}, \
                 capability_count={}",
                candidate.search_paths.len(),
                self.skill_catalog.base_skills().len(),
                self.capability_sync.current_capabilities().len()
            );

            Ok(candidate.search_paths)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use async_trait::async_trait;
    use serde_json::{Value, json};

    use super::*;
    use crate::bootstrap::deps::{
        core::{
            AstrError, CapabilityInvoker, CapabilityKind, CapabilitySpec, CapabilitySpecBuildError,
            LlmEventSink, LlmOutput, LlmProvider, LlmRequest, ModelLimits, PluginRegistry,
            PromptBuildOutput, PromptBuildRequest, PromptProvider, ResourceProvider,
            ResourceReadResult, ResourceRequestContext, Result, Tool, ToolContext, ToolDefinition,
            ToolExecutionResult,
        },
        kernel::{CapabilityRouter, Kernel, ToolCapabilityInvoker},
    };

    #[derive(Debug)]
    struct StaticTool {
        name: &'static str,
        tags: &'static [&'static str],
    }

    #[async_trait]
    impl Tool for StaticTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: self.name.to_string(),
                description: format!("tool {}", self.name),
                parameters: json!({"type": "object"}),
            }
        }

        fn capability_spec(&self) -> std::result::Result<CapabilitySpec, CapabilitySpecBuildError> {
            CapabilitySpec::builder(self.name, CapabilityKind::Tool)
                .description(format!("tool {}", self.name))
                .schema(json!({"type": "object"}), json!({"type": "string"}))
                .tags(self.tags.iter().copied())
                .build()
        }

        async fn execute(
            &self,
            tool_call_id: String,
            _input: Value,
            _ctx: &ToolContext,
        ) -> Result<ToolExecutionResult> {
            Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: self.name.to_string(),
                ok: true,
                output: String::new(),
                continuation: None,
                error: None,
                metadata: None,
                duration_ms: 0,
                truncated: false,
            })
        }
    }

    #[derive(Debug)]
    struct NoopLlmProvider;

    #[async_trait]
    impl LlmProvider for NoopLlmProvider {
        async fn generate(
            &self,
            _request: LlmRequest,
            _sink: Option<LlmEventSink>,
        ) -> Result<LlmOutput> {
            Err(AstrError::Validation(
                "noop llm provider should not execute in this test".to_string(),
            ))
        }

        fn model_limits(&self) -> ModelLimits {
            ModelLimits {
                context_window: 8192,
                max_output_tokens: 4096,
            }
        }
    }

    #[derive(Debug)]
    struct NoopPromptProvider;

    #[async_trait]
    impl PromptProvider for NoopPromptProvider {
        async fn build_prompt(&self, _request: PromptBuildRequest) -> Result<PromptBuildOutput> {
            Ok(PromptBuildOutput {
                system_prompt: "noop".to_string(),
                system_prompt_blocks: Vec::new(),
                prompt_cache_hints: Default::default(),
                cache_metrics: Default::default(),
                metadata: Value::Null,
            })
        }
    }

    #[derive(Debug)]
    struct NoopResourceProvider;

    #[async_trait]
    impl ResourceProvider for NoopResourceProvider {
        async fn read_resource(
            &self,
            _uri: &str,
            _context: &ResourceRequestContext,
        ) -> Result<ResourceReadResult> {
            Ok(ResourceReadResult {
                uri: "noop://resource".to_string(),
                content: Value::Null,
                metadata: Value::Null,
            })
        }
    }

    fn invoker(name: &'static str, tags: &'static [&'static str]) -> Arc<dyn CapabilityInvoker> {
        Arc::new(
            ToolCapabilityInvoker::new(Arc::new(StaticTool { name, tags }))
                .expect("static tool should build"),
        ) as Arc<dyn CapabilityInvoker>
    }

    fn test_kernel(builtin_invokers: &[Arc<dyn CapabilityInvoker>]) -> Arc<Kernel> {
        let mut builder = CapabilityRouter::builder();
        for invoker in builtin_invokers {
            builder = builder.register_invoker(Arc::clone(invoker));
        }
        let router = builder.build().expect("router should build");
        Arc::new(
            Kernel::builder()
                .with_capabilities(router)
                .with_llm_provider(Arc::new(NoopLlmProvider))
                .with_prompt_provider(Arc::new(NoopPromptProvider))
                .with_resource_provider(Arc::new(NoopResourceProvider))
                .build()
                .expect("kernel should build"),
        )
    }

    #[tokio::test]
    async fn governance_port_exposes_runtime_snapshot_and_shutdown() {
        let coordinator = Arc::new(RuntimeCoordinator::new(
            Arc::new(AppRuntimeHandle),
            Arc::new(PluginRegistry::default()),
            vec![
                CapabilitySpec::builder("test_tool", CapabilityKind::Tool)
                    .description("test")
                    .schema(
                        serde_json::json!({"type":"object"}),
                        serde_json::json!({"type":"string"}),
                    )
                    .build()
                    .expect("static capability should build"),
            ],
        ));
        let port = CoordinatorGovernancePort { coordinator };

        let snapshot = port.snapshot();
        assert_eq!(snapshot.runtime_name, "astrcode-application");
        assert_eq!(snapshot.runtime_kind, "application");
        assert_eq!(snapshot.capabilities.len(), 1);
        assert!(snapshot.plugins.is_empty());

        port.shutdown(1).await.expect("shutdown should succeed");
    }

    #[tokio::test]
    async fn rollback_restores_previous_mcp_and_plugin_external_layers() {
        let mcp_manager = McpConnectionManager::new();
        let alpha = McpServerConfig {
            name: "alpha".to_string(),
            transport: astrcode_adapter_mcp::config::McpTransportConfig::Stdio {
                command: "echo".to_string(),
                args: Vec::new(),
                env: HashMap::new(),
            },
            scope: astrcode_adapter_mcp::config::McpConfigScope::User,
            enabled: false,
            timeout_secs: 120,
            init_timeout_secs: 30,
            max_reconnect_attempts: 5,
        };
        let beta = McpServerConfig {
            name: "beta".to_string(),
            ..alpha.clone()
        };
        mcp_manager
            .reload_config(vec![alpha])
            .await
            .expect("alpha config should apply");

        let stable_local_invokers = vec![invoker("read_file", &["source:builtin"])];
        let kernel = test_kernel(&stable_local_invokers);
        let tool_search_index =
            Arc::new(astrcode_adapter_tools::builtin_tools::tool_search::ToolSearchIndex::new());
        let capability_sync = CapabilitySurfaceSync::new(
            kernel,
            stable_local_invokers,
            Arc::clone(&tool_search_index),
        );
        capability_sync
            .apply_external_invokers(vec![invoker("plugin.search", &["source:plugin"])])
            .expect("previous plugin surface should apply");

        let rollback = GovernanceReloadRollback::capture(&mcp_manager, &capability_sync).await;

        mcp_manager
            .reload_config(vec![beta])
            .await
            .expect("beta config should apply");
        capability_sync
            .apply_external_invokers(Vec::new())
            .expect("candidate external surface should apply");

        rollback
            .restore(&mcp_manager, &capability_sync)
            .await
            .expect("rollback should succeed");

        let declared_names = mcp_manager
            .current_surface()
            .await
            .server_statuses
            .into_iter()
            .map(|status| status.name)
            .collect::<Vec<_>>();
        assert_eq!(declared_names, vec!["alpha".to_string()]);
        let external_names = capability_sync
            .current_external_invokers()
            .into_iter()
            .map(|invoker| invoker.capability_spec().name.to_string())
            .collect::<Vec<_>>();
        assert_eq!(external_names, vec!["plugin.search".to_string()]);
    }
}
