use std::collections::BTreeMap;
use std::sync::RwLock;

use crate::{CapabilityDescriptor, PluginManifest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginState {
    Discovered,
    Initialized,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginHealth {
    Unknown,
    Healthy,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone)]
pub struct PluginEntry {
    pub manifest: PluginManifest,
    pub state: PluginState,
    pub health: PluginHealth,
    pub failure_count: u32,
    pub capabilities: Vec<CapabilityDescriptor>,
    pub failure: Option<String>,
    pub last_checked_at: Option<String>,
}

#[derive(Debug, Default)]
pub struct PluginRegistry {
    plugins: RwLock<BTreeMap<String, PluginEntry>>,
}

impl PluginRegistry {
    pub fn record_discovered(&self, manifest: PluginManifest) {
        self.upsert(PluginEntry {
            manifest,
            state: PluginState::Discovered,
            health: PluginHealth::Unknown,
            failure_count: 0,
            capabilities: Vec::new(),
            failure: None,
            last_checked_at: None,
        });
    }

    pub fn record_initialized(
        &self,
        manifest: PluginManifest,
        capabilities: Vec<CapabilityDescriptor>,
    ) {
        self.upsert(PluginEntry {
            manifest,
            state: PluginState::Initialized,
            health: PluginHealth::Healthy,
            failure_count: 0,
            capabilities,
            failure: None,
            last_checked_at: None,
        });
    }

    pub fn record_failed(
        &self,
        manifest: PluginManifest,
        failure: impl Into<String>,
        capabilities: Vec<CapabilityDescriptor>,
    ) {
        self.upsert(PluginEntry {
            manifest,
            state: PluginState::Failed,
            health: PluginHealth::Unavailable,
            failure_count: 1,
            capabilities,
            failure: Some(failure.into()),
            last_checked_at: None,
        });
    }

    pub fn get(&self, name: &str) -> Option<PluginEntry> {
        self.plugins
            .read()
            .expect("plugin registry lock poisoned")
            .get(name)
            .cloned()
    }

    pub fn snapshot(&self) -> Vec<PluginEntry> {
        self.plugins
            .read()
            .expect("plugin registry lock poisoned")
            .values()
            .cloned()
            .collect()
    }

    pub fn replace_snapshot(&self, entries: Vec<PluginEntry>) {
        let mut plugins = self.plugins.write().expect("plugin registry lock poisoned");
        plugins.clear();
        for entry in entries {
            plugins.insert(entry.manifest.name.clone(), entry);
        }
    }

    pub fn record_runtime_success(&self, name: &str, checked_at: String) {
        self.mutate(name, |entry| {
            if entry.state == PluginState::Initialized {
                entry.health = PluginHealth::Healthy;
            }
            entry.failure_count = 0;
            entry.failure = None;
            entry.last_checked_at = Some(checked_at);
        });
    }

    pub fn record_runtime_failure(
        &self,
        name: &str,
        failure: impl Into<String>,
        checked_at: String,
    ) {
        let failure = failure.into();
        self.mutate(name, |entry| {
            entry.failure_count = entry.failure_count.saturating_add(1);
            entry.failure = Some(failure.clone());
            entry.last_checked_at = Some(checked_at);
            if entry.state == PluginState::Initialized {
                entry.health = if entry.failure_count >= 3 {
                    PluginHealth::Unavailable
                } else {
                    PluginHealth::Degraded
                };
            } else {
                entry.health = PluginHealth::Unavailable;
            }
        });
    }

    pub fn record_health_probe(
        &self,
        name: &str,
        health: PluginHealth,
        failure: Option<String>,
        checked_at: String,
    ) {
        self.mutate(name, |entry| {
            entry.health = health.clone();
            if matches!(health, PluginHealth::Healthy) {
                entry.failure_count = 0;
                entry.failure = None;
            } else if let Some(message) = failure.clone() {
                entry.failure = Some(message);
            }
            entry.last_checked_at = Some(checked_at.clone());
        });
    }

    fn upsert(&self, entry: PluginEntry) {
        self.plugins
            .write()
            .expect("plugin registry lock poisoned")
            .insert(entry.manifest.name.clone(), entry);
    }

    fn mutate(&self, name: &str, update: impl FnOnce(&mut PluginEntry)) {
        if let Some(entry) = self
            .plugins
            .write()
            .expect("plugin registry lock poisoned")
            .get_mut(name)
        {
            update(entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use serde_json::json;

    use super::{PluginHealth, PluginRegistry, PluginState};
    use crate::{
        CapabilityDescriptor, CapabilityKind, PluginManifest, PluginType, SideEffectLevel,
        StabilityLevel,
    };

    fn manifest(name: &str) -> PluginManifest {
        PluginManifest {
            name: name.to_string(),
            version: "0.1.0".to_string(),
            description: format!("{name} manifest"),
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
            concurrency_safe: false,
            compact_clearable: false,
            profiles: vec!["coding".to_string()],
            tags: Vec::new(),
            permissions: Vec::new(),
            side_effect: SideEffectLevel::None,
            stability: StabilityLevel::Stable,
            metadata: json!(null),
        }
    }

    #[test]
    fn records_state_transitions_and_failure_details() {
        let registry = PluginRegistry::default();
        let manifest = manifest("repo-inspector");

        registry.record_discovered(manifest.clone());
        assert_eq!(
            registry
                .get("repo-inspector")
                .expect("entry should exist")
                .state,
            PluginState::Discovered
        );

        registry.record_initialized(manifest.clone(), vec![capability("tool.repo.inspect")]);
        let initialized = registry
            .get("repo-inspector")
            .expect("initialized entry should exist");
        assert_eq!(initialized.state, PluginState::Initialized);
        assert_eq!(initialized.health, PluginHealth::Healthy);
        assert_eq!(initialized.capabilities.len(), 1);
        assert!(initialized.failure.is_none());

        registry.record_failed(
            manifest,
            "capability conflict",
            vec![capability("tool.repo.inspect")],
        );
        let failed = registry
            .get("repo-inspector")
            .expect("failed entry should exist");
        assert_eq!(failed.state, PluginState::Failed);
        assert_eq!(failed.health, PluginHealth::Unavailable);
        assert_eq!(failed.failure.as_deref(), Some("capability conflict"));
        assert_eq!(failed.capabilities.len(), 1);
    }

    #[test]
    fn snapshot_is_sorted_by_plugin_name() {
        let registry = PluginRegistry::default();
        registry.record_discovered(manifest("zeta"));
        registry.record_discovered(manifest("alpha"));

        let snapshot = registry.snapshot();
        assert_eq!(
            snapshot
                .into_iter()
                .map(|entry| entry.manifest.name)
                .collect::<Vec<_>>(),
            vec!["alpha".to_string(), "zeta".to_string()]
        );
    }

    #[test]
    fn replace_snapshot_overwrites_existing_entries() {
        let registry = PluginRegistry::default();
        registry.record_discovered(manifest("alpha"));
        registry.replace_snapshot(vec![super::PluginEntry {
            manifest: manifest("beta"),
            state: PluginState::Initialized,
            health: PluginHealth::Healthy,
            failure_count: 0,
            capabilities: vec![capability("tool.beta")],
            failure: None,
            last_checked_at: None,
        }]);

        assert!(registry.get("alpha").is_none());
        assert_eq!(
            registry.get("beta").expect("beta should exist").state,
            PluginState::Initialized
        );
    }

    #[test]
    fn runtime_health_transitions_degrade_then_recover() {
        let registry = PluginRegistry::default();
        registry.record_initialized(manifest("alpha"), vec![capability("tool.alpha")]);

        registry.record_runtime_failure("alpha", "transport closed", Utc::now().to_rfc3339());
        let degraded = registry.get("alpha").expect("alpha should exist");
        assert_eq!(degraded.health, PluginHealth::Degraded);
        assert_eq!(degraded.failure_count, 1);

        registry.record_runtime_success("alpha", Utc::now().to_rfc3339());
        let healthy = registry.get("alpha").expect("alpha should still exist");
        assert_eq!(healthy.health, PluginHealth::Healthy);
        assert_eq!(healthy.failure_count, 0);
        assert!(healthy.failure.is_none());
    }
}
