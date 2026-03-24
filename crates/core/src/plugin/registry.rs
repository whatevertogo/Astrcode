use std::collections::HashMap;

use crate::PluginManifest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginState {
    Discovered,
    Initialized,
    Failed,
}

#[derive(Debug, Clone)]
pub struct PluginEntry {
    pub manifest: PluginManifest,
    pub state: PluginState,
}

#[derive(Debug, Default)]
pub struct PluginRegistry {
    plugins: HashMap<String, PluginEntry>,
}

impl PluginRegistry {
    pub fn register(&mut self, manifest: PluginManifest, state: PluginState) {
        self.plugins
            .insert(manifest.name.clone(), PluginEntry { manifest, state });
    }

    pub fn get(&self, name: &str) -> Option<&PluginEntry> {
        self.plugins.get(name)
    }

    pub fn all(&self) -> impl Iterator<Item = &PluginEntry> {
        self.plugins.values()
    }
}
