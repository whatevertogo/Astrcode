//! # 运行时协调器
//!
//! 组合根拥有的运行时设施：统一管理活跃 runtime、插件快照、能力表面与托管组件生命周期。

use std::sync::{Arc, RwLock};

use astrcode_plugin_host::{PluginEntry, PluginRegistry};

use super::deps::core::{
    AstrError, CapabilitySpec, ManagedRuntimeComponent, Result, RuntimeHandle, support,
};

/// 运行时协调器。
///
/// 这是 server 组合根的设施 owner，而不是应用层业务对象。
pub(crate) struct RuntimeCoordinator {
    active_runtime: Arc<dyn RuntimeHandle>,
    plugin_registry: Arc<PluginRegistry>,
    capabilities: RwLock<Arc<[CapabilitySpec]>>,
    managed_components: RwLock<Vec<Arc<dyn ManagedRuntimeComponent>>>,
}

impl std::fmt::Debug for RuntimeCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeCoordinator")
            .field("runtime_name", &self.active_runtime.runtime_name())
            .field("runtime_kind", &self.active_runtime.runtime_kind())
            .finish_non_exhaustive()
    }
}

impl RuntimeCoordinator {
    pub(crate) fn new(
        active_runtime: Arc<dyn RuntimeHandle>,
        plugin_registry: Arc<PluginRegistry>,
        capabilities: Vec<CapabilitySpec>,
    ) -> Self {
        Self {
            active_runtime,
            plugin_registry,
            capabilities: RwLock::new(Arc::from(capabilities)),
            managed_components: RwLock::new(Vec::new()),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn with_managed_components(
        self,
        managed_components: Vec<Arc<dyn ManagedRuntimeComponent>>,
    ) -> Self {
        support::with_write_lock_recovery(
            &self.managed_components,
            "runtime coordinator managed components",
            |components| *components = managed_components,
        );
        self
    }

    pub(crate) fn runtime(&self) -> Arc<dyn RuntimeHandle> {
        Arc::clone(&self.active_runtime)
    }

    pub(crate) fn plugin_registry(&self) -> Arc<PluginRegistry> {
        Arc::clone(&self.plugin_registry)
    }

    pub(crate) fn capabilities(&self) -> Vec<CapabilitySpec> {
        support::with_read_lock_recovery(
            &self.capabilities,
            "runtime coordinator capabilities",
            |capabilities| capabilities.iter().cloned().collect(),
        )
    }

    #[allow(dead_code)]
    pub(crate) fn managed_components(&self) -> Vec<Arc<dyn ManagedRuntimeComponent>> {
        support::with_read_lock_recovery(
            &self.managed_components,
            "runtime coordinator managed components",
            Clone::clone,
        )
    }

    pub(crate) fn replace_runtime_surface(
        &self,
        plugin_entries: Vec<PluginEntry>,
        capabilities: Vec<CapabilitySpec>,
        managed_components: Vec<Arc<dyn ManagedRuntimeComponent>>,
    ) -> Vec<Arc<dyn ManagedRuntimeComponent>> {
        self.plugin_registry.replace_snapshot(plugin_entries);
        support::with_write_lock_recovery(
            &self.capabilities,
            "runtime coordinator capabilities",
            |current_capabilities| *current_capabilities = Arc::from(capabilities),
        );
        support::with_write_lock_recovery(
            &self.managed_components,
            "runtime coordinator managed components",
            |current_components| std::mem::replace(current_components, managed_components),
        )
    }

    pub(crate) async fn shutdown(&self, timeout_secs: u64) -> Result<()> {
        let mut failures = Vec::new();

        if let Err(error) = self.active_runtime.shutdown(timeout_secs).await {
            log::error!(
                "failed to shut down runtime '{}' (kind '{}'): {}",
                self.active_runtime.runtime_name(),
                self.active_runtime.runtime_kind(),
                error
            );
            failures.push(format!(
                "runtime '{}' failed to shut down: {}",
                self.active_runtime.runtime_name(),
                error
            ));
        }

        let managed_components = support::with_read_lock_recovery(
            &self.managed_components,
            "runtime coordinator managed components",
            Clone::clone,
        );

        for component in managed_components {
            if let Err(error) = component.shutdown_component().await {
                let component_name = component.component_name();
                log::error!(
                    "failed to shut down managed component '{}': {}",
                    component_name,
                    error
                );
                failures.push(format!(
                    "managed component '{}' failed to shut down: {}",
                    component_name, error
                ));
            }
        }

        if failures.is_empty() {
            Ok(())
        } else {
            Err(AstrError::Internal(failures.join("; ")))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use astrcode_plugin_host::{PluginEntry, PluginHealth, PluginRegistry, PluginState};
    use async_trait::async_trait;
    use serde_json::json;

    use super::RuntimeCoordinator;
    use crate::bootstrap::deps::core::{
        AstrError, CapabilityKind, CapabilitySpec, InvocationMode, ManagedRuntimeComponent, Result,
        RuntimeHandle, SideEffect, Stability,
    };

    struct FakeRuntimeHandle {
        events: Arc<Mutex<Vec<String>>>,
        fail: bool,
    }

    #[async_trait]
    impl RuntimeHandle for FakeRuntimeHandle {
        fn runtime_name(&self) -> &'static str {
            "test-runtime"
        }

        fn runtime_kind(&self) -> &'static str {
            "unit-test"
        }

        async fn shutdown(&self, _timeout_secs: u64) -> Result<()> {
            self.events
                .lock()
                .expect("events lock")
                .push("runtime".to_string());
            if self.fail {
                Err(AstrError::Internal("runtime failure".to_string()))
            } else {
                Ok(())
            }
        }
    }

    struct FakeManagedComponent {
        name: &'static str,
        events: Arc<Mutex<Vec<String>>>,
        fail: bool,
    }

    #[async_trait]
    impl ManagedRuntimeComponent for FakeManagedComponent {
        fn component_name(&self) -> String {
            self.name.to_string()
        }

        async fn shutdown_component(&self) -> Result<()> {
            self.events
                .lock()
                .expect("events lock")
                .push(self.name.to_string());
            if self.fail {
                Err(AstrError::Internal(format!("{} failure", self.name)))
            } else {
                Ok(())
            }
        }
    }

    fn capability(name: &str) -> CapabilitySpec {
        CapabilitySpec {
            name: name.into(),
            kind: CapabilityKind::Tool,
            description: format!("{name} capability"),
            input_schema: json!({ "type": "object" }),
            output_schema: json!({ "type": "object" }),
            invocation_mode: InvocationMode::Unary,
            concurrency_safe: false,
            compact_clearable: false,
            profiles: vec!["coding".to_string()],
            tags: Vec::new(),
            permissions: Vec::new(),
            side_effect: SideEffect::None,
            stability: Stability::Stable,
            metadata: json!(null),
            max_result_inline_size: None,
        }
    }

    #[tokio::test]
    async fn shutdown_runs_runtime_before_managed_components() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let coordinator = RuntimeCoordinator::new(
            Arc::new(FakeRuntimeHandle {
                events: Arc::clone(&events),
                fail: false,
            }),
            Arc::new(PluginRegistry::default()),
            vec![capability("tool.sample")],
        )
        .with_managed_components(vec![
            Arc::new(FakeManagedComponent {
                name: "plugin-a",
                events: Arc::clone(&events),
                fail: false,
            }),
            Arc::new(FakeManagedComponent {
                name: "plugin-b",
                events: Arc::clone(&events),
                fail: false,
            }),
        ]);

        coordinator.shutdown(3).await.expect("shutdown should pass");

        assert_eq!(
            events.lock().expect("events lock").clone(),
            vec![
                "runtime".to_string(),
                "plugin-a".to_string(),
                "plugin-b".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn shutdown_collects_failures_after_attempting_every_component() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let coordinator = RuntimeCoordinator::new(
            Arc::new(FakeRuntimeHandle {
                events: Arc::clone(&events),
                fail: true,
            }),
            Arc::new(PluginRegistry::default()),
            vec![capability("tool.sample")],
        )
        .with_managed_components(vec![
            Arc::new(FakeManagedComponent {
                name: "plugin-a",
                events: Arc::clone(&events),
                fail: true,
            }),
            Arc::new(FakeManagedComponent {
                name: "plugin-b",
                events,
                fail: false,
            }),
        ]);

        let error = coordinator
            .shutdown(3)
            .await
            .expect_err("shutdown should bubble failure");

        let message = error.to_string();
        assert!(message.contains("runtime 'test-runtime' failed to shut down"));
        assert!(message.contains("managed component 'plugin-a' failed to shut down"));
    }

    #[test]
    fn replace_runtime_surface_swaps_registry_capabilities_and_components() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let registry = Arc::new(PluginRegistry::default());
        registry.record_discovered(astrcode_plugin_host::PluginManifest {
            name: "alpha".to_string(),
            version: "0.1.0".to_string(),
            description: "alpha".to_string(),
            plugin_type: vec![astrcode_plugin_host::PluginType::Tool],
            capabilities: Vec::new(),
            executable: Some("alpha.exe".to_string()),
            args: Vec::new(),
            working_dir: None,
            repository: None,
            resources: Vec::new(),
            commands: Vec::new(),
            themes: Vec::new(),
            prompts: Vec::new(),
            providers: Vec::new(),
            skills: Vec::new(),
        });
        let coordinator = RuntimeCoordinator::new(
            Arc::new(FakeRuntimeHandle {
                events: Arc::clone(&events),
                fail: false,
            }),
            Arc::clone(&registry),
            vec![capability("tool.alpha")],
        )
        .with_managed_components(vec![Arc::new(FakeManagedComponent {
            name: "plugin-a",
            events,
            fail: false,
        })]);

        let old = coordinator.replace_runtime_surface(
            vec![PluginEntry {
                manifest: astrcode_plugin_host::PluginManifest {
                    name: "beta".to_string(),
                    version: "0.2.0".to_string(),
                    description: "beta".to_string(),
                    plugin_type: vec![astrcode_plugin_host::PluginType::Tool],
                    capabilities: Vec::new(),
                    executable: Some("beta.exe".to_string()),
                    args: Vec::new(),
                    working_dir: None,
                    repository: None,
                    resources: Vec::new(),
                    commands: Vec::new(),
                    themes: Vec::new(),
                    prompts: Vec::new(),
                    providers: Vec::new(),
                    skills: Vec::new(),
                },
                state: PluginState::Initialized,
                health: PluginHealth::Healthy,
                failure_count: 0,
                capabilities: vec![capability("tool.beta")],
                failure: None,
                warnings: Vec::new(),
                last_checked_at: None,
            }],
            vec![capability("tool.beta")],
            Vec::new(),
        );

        assert_eq!(old.len(), 1);
        assert_eq!(
            coordinator
                .plugin_registry()
                .snapshot()
                .into_iter()
                .map(|entry| entry.manifest.name)
                .collect::<Vec<_>>(),
            vec!["beta".to_string()]
        );
        assert_eq!(
            coordinator
                .capabilities()
                .into_iter()
                .map(|descriptor| descriptor.name.to_string())
                .collect::<Vec<_>>(),
            vec!["tool.beta".to_string()]
        );
    }
}
