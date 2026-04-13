//! # 治理装配
//!
//! 负责把底层 `RuntimeCoordinator` 适配成应用层治理端口，
//! 同时承载组合根需要的默认观测与会话信息桥接。

use std::sync::Arc;

use astrcode_application::{
    AppGovernance, ApplicationError, ObservabilitySnapshotProvider, RuntimeGovernancePort,
    RuntimeGovernanceSnapshot, RuntimeObservabilitySnapshot, SessionInfoProvider,
    lifecycle::TaskRegistry,
};
use astrcode_core::{
    CapabilitySpec, ManagedRuntimeComponent, PluginRegistry, RuntimeCoordinator, RuntimeHandle,
};
use astrcode_kernel::Kernel;
use astrcode_plugin::Supervisor;
use astrcode_session_runtime::SessionRuntime;
use async_trait::async_trait;

pub(crate) fn build_app_governance(
    session_runtime: Arc<SessionRuntime>,
    kernel: Arc<Kernel>,
    plugin_registry: Arc<PluginRegistry>,
    plugin_supervisors: Vec<Arc<Supervisor>>,
) -> Arc<AppGovernance> {
    let managed_components: Vec<Arc<dyn ManagedRuntimeComponent>> = plugin_supervisors
        .into_iter()
        .map(|supervisor| supervisor as Arc<dyn ManagedRuntimeComponent>)
        .collect();
    let coordinator = Arc::new(
        RuntimeCoordinator::new(Arc::new(AppRuntimeHandle), plugin_registry, Vec::new())
            .with_managed_components(managed_components),
    );

    Arc::new(AppGovernance::new(
        Arc::new(CoordinatorGovernancePort {
            coordinator,
            capabilities: Arc::new(KernelCapabilitySnapshotSource { kernel }),
        }),
        Arc::new(TaskRegistry::new()),
        Arc::new(DefaultObservability),
        Arc::new(SessionRuntimeInfo { session_runtime }),
    ))
}

trait CapabilitySnapshotSource: Send + Sync {
    fn capability_specs(&self) -> Vec<CapabilitySpec>;
}

struct KernelCapabilitySnapshotSource {
    kernel: Arc<Kernel>,
}

impl CapabilitySnapshotSource for KernelCapabilitySnapshotSource {
    fn capability_specs(&self) -> Vec<CapabilitySpec> {
        self.kernel.surface().snapshot().capability_specs
    }
}

struct CoordinatorGovernancePort {
    coordinator: Arc<RuntimeCoordinator>,
    capabilities: Arc<dyn CapabilitySnapshotSource>,
}

impl std::fmt::Debug for CoordinatorGovernancePort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoordinatorGovernancePort")
            .finish_non_exhaustive()
    }
}

impl RuntimeGovernancePort for CoordinatorGovernancePort {
    fn snapshot(&self) -> RuntimeGovernanceSnapshot {
        let runtime = self.coordinator.runtime();
        RuntimeGovernanceSnapshot {
            runtime_name: runtime.runtime_name().to_string(),
            runtime_kind: runtime.runtime_kind().to_string(),
            capabilities: self.capabilities.capability_specs(),
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
struct AppRuntimeHandle;

#[async_trait]
impl RuntimeHandle for AppRuntimeHandle {
    fn runtime_name(&self) -> &'static str {
        "astrcode-application"
    }

    fn runtime_kind(&self) -> &'static str {
        "application"
    }

    async fn shutdown(
        &self,
        _timeout_secs: u64,
    ) -> std::result::Result<(), astrcode_core::AstrError> {
        Ok(())
    }
}

#[derive(Debug)]
struct DefaultObservability;

impl ObservabilitySnapshotProvider for DefaultObservability {
    fn snapshot(&self) -> RuntimeObservabilitySnapshot {
        RuntimeObservabilitySnapshot::default()
    }
}

struct SessionRuntimeInfo {
    session_runtime: Arc<SessionRuntime>,
}

impl std::fmt::Debug for SessionRuntimeInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionRuntimeInfo").finish()
    }
}

impl SessionInfoProvider for SessionRuntimeInfo {
    fn loaded_session_count(&self) -> usize {
        self.session_runtime.list_sessions().len()
    }

    fn running_session_ids(&self) -> Vec<String> {
        self.session_runtime
            .list_sessions()
            .into_iter()
            .map(|id| id.to_string())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::CapabilityKind;

    use super::*;

    struct StaticCapabilitySnapshotSource;

    impl CapabilitySnapshotSource for StaticCapabilitySnapshotSource {
        fn capability_specs(&self) -> Vec<CapabilitySpec> {
            vec![
                CapabilitySpec::builder("test_tool", CapabilityKind::Tool)
                    .description("test")
                    .schema(
                        serde_json::json!({"type":"object"}),
                        serde_json::json!({"type":"string"}),
                    )
                    .build()
                    .expect("static capability should build"),
            ]
        }
    }

    #[tokio::test]
    async fn governance_port_exposes_runtime_snapshot_and_shutdown() {
        let port = CoordinatorGovernancePort {
            coordinator: Arc::new(RuntimeCoordinator::new(
                Arc::new(AppRuntimeHandle),
                Arc::new(PluginRegistry::default()),
                Vec::new(),
            )),
            capabilities: Arc::new(StaticCapabilitySnapshotSource),
        };

        let snapshot = port.snapshot();
        assert_eq!(snapshot.runtime_name, "astrcode-application");
        assert_eq!(snapshot.runtime_kind, "application");
        assert_eq!(snapshot.capabilities.len(), 1);
        assert!(snapshot.plugins.is_empty());

        port.shutdown(1).await.expect("shutdown should succeed");
    }
}
