use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use astrcode_core::{
    CapabilityContext, CapabilityDescriptor, CapabilityExecutionResult, CapabilityInvoker,
    CapabilityKind, ManagedRuntimeComponent, PluginHealth, PluginState, PluginType, Result,
    SideEffectLevel, StabilityLevel,
};
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::bootstrap::{bootstrap_runtime_from_manifests, RuntimeBootstrap};
use crate::runtime_surface_assembler::{
    conflicting_capability_name, LoadedPlugin, ManagedPluginComponent, ManagedPluginHealth,
    PluginInitializer,
};
use crate::test_support::TestEnvGuard;

struct FakeInitializer {
    responses: HashMap<String, FakePluginResponse>,
}

enum FakePluginResponse {
    Loaded(LoadedPlugin),
    Failed(String),
}

#[async_trait]
impl PluginInitializer for FakeInitializer {
    async fn initialize(
        &self,
        manifest: &astrcode_core::PluginManifest,
    ) -> std::result::Result<LoadedPlugin, astrcode_core::AstrError> {
        match self
            .responses
            .get(&manifest.name)
            .expect("initializer response should exist")
        {
            FakePluginResponse::Loaded(loaded) => Ok(LoadedPlugin {
                component: loaded.component.clone(),
                capabilities: loaded.capabilities.clone(),
                invokers: loaded.invokers.clone(),
            }),
            FakePluginResponse::Failed(message) => {
                Err(astrcode_core::AstrError::Internal(message.clone()))
            }
        }
    }
}

struct FakeManagedComponent {
    name: String,
    shutdowns: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl ManagedRuntimeComponent for FakeManagedComponent {
    fn component_name(&self) -> String {
        self.name.clone()
    }

    async fn shutdown_component(&self) -> Result<()> {
        self.shutdowns
            .lock()
            .expect("shutdown log lock")
            .push(self.name.clone());
        Ok(())
    }
}

#[async_trait]
impl ManagedPluginComponent for FakeManagedComponent {
    async fn health_report(
        &self,
    ) -> std::result::Result<ManagedPluginHealth, astrcode_core::AstrError> {
        Ok(ManagedPluginHealth {
            health: PluginHealth::Healthy,
            message: None,
        })
    }
}

struct FakeCapabilityInvoker {
    descriptor: CapabilityDescriptor,
}

#[async_trait]
impl CapabilityInvoker for FakeCapabilityInvoker {
    fn descriptor(&self) -> CapabilityDescriptor {
        self.descriptor.clone()
    }

    async fn invoke(
        &self,
        _payload: Value,
        _ctx: &CapabilityContext,
    ) -> Result<CapabilityExecutionResult> {
        Ok(CapabilityExecutionResult::ok(
            self.descriptor.name.clone(),
            Value::Null,
        ))
    }
}

fn manifest(name: &str) -> astrcode_core::PluginManifest {
    astrcode_core::PluginManifest {
        name: name.to_string(),
        version: "0.1.0".to_string(),
        description: format!("{name} plugin"),
        plugin_type: vec![PluginType::Tool],
        capabilities: Vec::new(),
        executable: Some("plugin.exe".to_string()),
        args: Vec::new(),
        working_dir: None,
        repository: None,
    }
}

fn capability(name: &str) -> CapabilityDescriptor {
    CapabilityDescriptor {
        name: name.to_string(),
        kind: CapabilityKind::tool(),
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

fn loaded_plugin(
    plugin_name: &str,
    capability_names: &[&str],
    shutdowns: Arc<Mutex<Vec<String>>>,
) -> LoadedPlugin {
    let capabilities = capability_names
        .iter()
        .map(|name| capability(name))
        .collect::<Vec<_>>();
    let invokers = capabilities
        .iter()
        .cloned()
        .map(|descriptor| {
            Arc::new(FakeCapabilityInvoker { descriptor }) as Arc<dyn CapabilityInvoker>
        })
        .collect();

    LoadedPlugin {
        component: Arc::new(FakeManagedComponent {
            name: plugin_name.to_string(),
            shutdowns,
        }),
        capabilities,
        invokers,
    }
}

fn bootstrap_from(
    manifests: Vec<astrcode_core::PluginManifest>,
    initializer: FakeInitializer,
) -> (RuntimeBootstrap, TestEnvGuard) {
    let guard = TestEnvGuard::new();
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime should build");
    let bootstrap = runtime
        .block_on(async { bootstrap_runtime_from_manifests(manifests, &initializer).await })
        .expect("runtime bootstrap should succeed");
    (bootstrap, guard)
}

#[test]
fn bootstrap_without_plugins_keeps_builtin_capabilities() {
    let initializer = FakeInitializer {
        responses: Default::default(),
    };
    let (bootstrap, _guard) = bootstrap_from(Vec::new(), initializer);

    assert!(bootstrap
        .coordinator
        .capabilities()
        .iter()
        .any(|descriptor| descriptor.name == "shell"));
    assert!(bootstrap
        .coordinator
        .plugin_registry()
        .snapshot()
        .is_empty());
}

#[test]
fn bootstrap_records_initialized_and_failed_plugins_without_aborting_server() {
    let shutdowns = Arc::new(Mutex::new(Vec::new()));
    let initializer = FakeInitializer {
        responses: HashMap::from([
            (
                "alpha".to_string(),
                FakePluginResponse::Loaded(loaded_plugin(
                    "alpha",
                    &["tool.alpha"],
                    Arc::clone(&shutdowns),
                )),
            ),
            (
                "beta".to_string(),
                FakePluginResponse::Failed("handshake failed".to_string()),
            ),
        ]),
    };

    let (bootstrap, _guard) =
        bootstrap_from(vec![manifest("alpha"), manifest("beta")], initializer);
    let registry = bootstrap.coordinator.plugin_registry();
    let alpha = registry.get("alpha").expect("alpha entry should exist");
    let beta = registry.get("beta").expect("beta entry should exist");

    assert_eq!(alpha.state, PluginState::Initialized);
    assert_eq!(alpha.capabilities.len(), 1);
    assert_eq!(beta.state, PluginState::Failed);
    assert_eq!(
        beta.failure.as_deref(),
        Some("internal error: handshake failed")
    );
    assert!(bootstrap
        .coordinator
        .capabilities()
        .iter()
        .any(|descriptor| descriptor.name == "tool.alpha"));
}

#[test]
fn bootstrap_rejects_duplicate_plugin_capabilities_deterministically() {
    let shutdowns = Arc::new(Mutex::new(Vec::new()));
    let initializer = FakeInitializer {
        responses: HashMap::from([
            (
                "alpha".to_string(),
                FakePluginResponse::Loaded(loaded_plugin(
                    "alpha",
                    &["tool.shared"],
                    Arc::clone(&shutdowns),
                )),
            ),
            (
                "beta".to_string(),
                FakePluginResponse::Loaded(loaded_plugin(
                    "beta",
                    &["tool.shared"],
                    Arc::clone(&shutdowns),
                )),
            ),
        ]),
    };

    let (bootstrap, _guard) =
        bootstrap_from(vec![manifest("beta"), manifest("alpha")], initializer);
    let snapshot = bootstrap.coordinator.plugin_registry().snapshot();
    let alpha = snapshot
        .iter()
        .find(|entry| entry.manifest.name == "alpha")
        .expect("alpha entry should exist");
    let beta = snapshot
        .iter()
        .find(|entry| entry.manifest.name == "beta")
        .expect("beta entry should exist");

    assert_eq!(alpha.state, PluginState::Initialized);
    assert_eq!(beta.state, PluginState::Failed);
    assert_eq!(
        shutdowns.lock().expect("shutdown log").clone(),
        vec!["beta".to_string()]
    );
}

#[test]
fn bootstrap_marks_plugin_failed_when_descriptor_is_invalid() {
    let shutdowns = Arc::new(Mutex::new(Vec::new()));
    let mut invalid = capability("tool.invalid");
    invalid.kind = CapabilityKind::custom("   ");
    let initializer = FakeInitializer {
        responses: HashMap::from([(
            "alpha".to_string(),
            FakePluginResponse::Loaded(LoadedPlugin {
                component: Arc::new(FakeManagedComponent {
                    name: "alpha".to_string(),
                    shutdowns: Arc::clone(&shutdowns),
                }),
                invokers: vec![Arc::new(FakeCapabilityInvoker {
                    descriptor: invalid.clone(),
                }) as Arc<dyn CapabilityInvoker>],
                capabilities: vec![invalid],
            }),
        )]),
    };

    let (bootstrap, _guard) = bootstrap_from(vec![manifest("alpha")], initializer);
    let alpha = bootstrap
        .coordinator
        .plugin_registry()
        .get("alpha")
        .expect("alpha entry should exist");

    assert_eq!(alpha.state, PluginState::Failed);
    assert_eq!(alpha.health, PluginHealth::Unavailable);
    assert!(alpha
        .failure
        .as_deref()
        .is_some_and(|message| message.contains("invalid")));
    assert_eq!(
        shutdowns.lock().expect("shutdown log").clone(),
        vec!["alpha".to_string()]
    );
}

#[tokio::test]
async fn governance_reload_swaps_runtime_surface_and_shutdowns_retired_plugins() {
    let _guard = TestEnvGuard::new();
    let shutdowns = Arc::new(Mutex::new(Vec::new()));
    let initial = FakeInitializer {
        responses: HashMap::from([(
            "alpha".to_string(),
            FakePluginResponse::Loaded(loaded_plugin(
                "alpha",
                &["tool.alpha"],
                Arc::clone(&shutdowns),
            )),
        )]),
    };
    let bootstrap = bootstrap_runtime_from_manifests(vec![manifest("alpha")], &initial)
        .await
        .expect("initial bootstrap should succeed");

    let replacement = FakeInitializer {
        responses: HashMap::from([(
            "beta".to_string(),
            FakePluginResponse::Loaded(loaded_plugin(
                "beta",
                &["tool.beta"],
                Arc::clone(&shutdowns),
            )),
        )]),
    };
    let reload = bootstrap
        .governance
        .reload_from_manifests(
            vec![manifest("beta")],
            &replacement,
            vec![PathBuf::from("plugins")],
        )
        .await
        .expect("reload should succeed");

    assert_eq!(
        bootstrap
            .coordinator
            .plugin_registry()
            .snapshot()
            .into_iter()
            .map(|entry| entry.manifest.name)
            .collect::<Vec<_>>(),
        vec!["beta".to_string()]
    );
    assert_eq!(
        bootstrap
            .coordinator
            .capabilities()
            .into_iter()
            .map(|descriptor| descriptor.name)
            .filter(|name| name.starts_with("tool."))
            .collect::<Vec<_>>(),
        vec!["tool.beta".to_string()]
    );
    assert_eq!(
        shutdowns.lock().expect("shutdown log").clone(),
        vec!["alpha".to_string()]
    );
    assert_eq!(
        reload.snapshot.plugin_search_paths,
        vec![PathBuf::from("plugins")]
    );
}

#[test]
fn conflicting_capability_name_detects_existing_and_local_duplicates() {
    let registered = std::collections::HashSet::from(["tool.shared".to_string()]);
    assert_eq!(
        conflicting_capability_name(&registered, &[capability("tool.shared")]),
        Some("tool.shared".to_string())
    );
    assert_eq!(
        conflicting_capability_name(
            &std::collections::HashSet::new(),
            &[capability("tool.local"), capability("tool.local")]
        ),
        Some("tool.local".to_string())
    );
}
