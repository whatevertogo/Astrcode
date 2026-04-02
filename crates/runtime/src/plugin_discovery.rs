use std::path::PathBuf;

use astrcode_core::{env::ASTRCODE_PLUGIN_DIRS_ENV, AstrError, PluginManifest};
use astrcode_plugin::PluginLoader;

pub(crate) fn configured_plugin_paths() -> Vec<PathBuf> {
    match std::env::var_os(ASTRCODE_PLUGIN_DIRS_ENV) {
        Some(raw_paths) => std::env::split_paths(&raw_paths).collect(),
        None => Vec::new(),
    }
}

pub(crate) fn discover_plugin_manifests_in(
    search_paths: &[PathBuf],
) -> std::result::Result<Vec<PluginManifest>, AstrError> {
    if search_paths.is_empty() {
        return Ok(Vec::new());
    }
    PluginLoader {
        search_paths: search_paths.to_vec(),
    }
    .discover()
}
