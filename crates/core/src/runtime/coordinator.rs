use std::sync::{Arc, RwLock};

use crate::{
    plugin::PluginEntry, AstrError, CapabilityDescriptor, ManagedRuntimeComponent, PluginRegistry,
    Result, RuntimeHandle,
};

pub struct RuntimeCoordinator {
    active_runtime: Arc<dyn RuntimeHandle>,
    plugin_registry: Arc<PluginRegistry>,
    capabilities: RwLock<Arc<[CapabilityDescriptor]>>,
    managed_components: RwLock<Vec<Arc<dyn ManagedRuntimeComponent>>>,
}

impl RuntimeCoordinator {
    pub fn new(
        active_runtime: Arc<dyn RuntimeHandle>,
        plugin_registry: Arc<PluginRegistry>,
        capabilities: Vec<CapabilityDescriptor>,
    ) -> Self {
        Self {
            active_runtime,
            plugin_registry,
            capabilities: RwLock::new(Arc::from(capabilities)),
            managed_components: RwLock::new(Vec::new()),
        }
    }

    pub fn with_managed_components(
        self,
        managed_components: Vec<Arc<dyn ManagedRuntimeComponent>>,
    ) -> Self {
        *self
            .managed_components
            .write()
            .expect("runtime coordinator managed components lock poisoned") = managed_components;
        self
    }

    pub fn runtime(&self) -> Arc<dyn RuntimeHandle> {
        Arc::clone(&self.active_runtime)
    }

    pub fn plugin_registry(&self) -> Arc<PluginRegistry> {
        Arc::clone(&self.plugin_registry)
    }

    pub fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        self.capabilities
            .read()
            .expect("runtime coordinator capabilities lock poisoned")
            .iter()
            .cloned()
            .collect()
    }

    pub fn replace_runtime_surface(
        &self,
        plugin_entries: Vec<PluginEntry>,
        capabilities: Vec<CapabilityDescriptor>,
        managed_components: Vec<Arc<dyn ManagedRuntimeComponent>>,
    ) -> Vec<Arc<dyn ManagedRuntimeComponent>> {
        self.plugin_registry.replace_snapshot(plugin_entries);
        *self
            .capabilities
            .write()
            .expect("runtime coordinator capabilities lock poisoned") = Arc::from(capabilities);
        let mut guard = self
            .managed_components
            .write()
            .expect("runtime coordinator managed components lock poisoned");
        std::mem::replace(&mut *guard, managed_components)
    }

    pub async fn shutdown(&self, timeout_secs: u64) -> Result<()> {
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

        // Keep the shutdown order deterministic so tests and operational logs can explain
        // exactly which managed component was closed after the runtime stopped accepting work.
        let managed_components = self
            .managed_components
            .read()
            .expect("runtime coordinator managed components lock poisoned")
            .clone();

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

    use async_trait::async_trait;
    use serde_json::json;

    use super::RuntimeCoordinator;
    use crate::{
        plugin::{PluginEntry, PluginHealth},
        AstrError, CapabilityDescriptor, CapabilityKind, ManagedRuntimeComponent, PluginRegistry,
        Result, RuntimeHandle, SideEffectLevel, StabilityLevel,
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

    fn capability(name: &str) -> CapabilityDescriptor {
        CapabilityDescriptor {
            name: name.to_string(),
            kind: CapabilityKind::Tool,
            description: format!("{name} capability"),
            input_schema: json!({ "type": "object" }),
            output_schema: json!({ "type": "object" }),
            streaming: false,
            profiles: vec!["coding".to_string()],
            tags: Vec::new(),
            permissions: Vec::new(),
            side_effect: SideEffectLevel::None,
            stability: StabilityLevel::Stable,
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
        registry.record_discovered(crate::PluginManifest {
            name: "alpha".to_string(),
            version: "0.1.0".to_string(),
            description: "alpha".to_string(),
            plugin_type: vec![crate::PluginType::Tool],
            capabilities: Vec::new(),
            executable: Some("alpha.exe".to_string()),
            args: Vec::new(),
            working_dir: None,
            repository: None,
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
                manifest: crate::PluginManifest {
                    name: "beta".to_string(),
                    version: "0.2.0".to_string(),
                    description: "beta".to_string(),
                    plugin_type: vec![crate::PluginType::Tool],
                    capabilities: Vec::new(),
                    executable: Some("beta.exe".to_string()),
                    args: Vec::new(),
                    working_dir: None,
                    repository: None,
                },
                state: crate::PluginState::Initialized,
                health: PluginHealth::Healthy,
                failure_count: 0,
                capabilities: vec![capability("tool.beta")],
                failure: None,
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
                .map(|descriptor| descriptor.name)
                .collect::<Vec<_>>(),
            vec!["tool.beta".to_string()]
        );
    }
}
