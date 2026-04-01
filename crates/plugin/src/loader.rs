use std::path::PathBuf;

use astrcode_core::{PluginManifest, Result};
use astrcode_protocol::plugin::{InitializeMessage, PeerDescriptor};

use crate::{PluginProcess, Supervisor};

#[derive(Debug, Default, Clone)]
pub struct PluginLoader {
    pub search_paths: Vec<PathBuf>,
}

impl PluginLoader {
    pub fn discover(&self) -> Result<Vec<PluginManifest>> {
        let mut manifests = Vec::new();
        for search_path in &self.search_paths {
            if !search_path.exists() {
                continue;
            }

            let entries = match std::fs::read_dir(search_path) {
                Ok(entries) => entries,
                Err(error) => {
                    log::warn!(
                        "skipping plugin directory '{}' because it could not be read: {}",
                        search_path.display(),
                        error
                    );
                    continue;
                }
            };

            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        log::warn!(
                            "skipping plugin entry in '{}' because it could not be inspected: {}",
                            search_path.display(),
                            error
                        );
                        continue;
                    }
                };
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
                    continue;
                }
                let raw = match std::fs::read_to_string(&path) {
                    Ok(raw) => raw,
                    Err(error) => {
                        log::warn!(
                            "skipping plugin manifest '{}' because it could not be read: {}",
                            path.display(),
                            error
                        );
                        continue;
                    }
                };
                let mut manifest = match PluginManifest::from_toml(&raw) {
                    Ok(manifest) => manifest,
                    Err(error) => {
                        log::warn!(
                            "skipping plugin manifest '{}' because it could not be parsed: {}",
                            path.display(),
                            error
                        );
                        continue;
                    }
                };
                if let Some(working_dir) = manifest.working_dir.clone() {
                    let working_dir_path = PathBuf::from(&working_dir);
                    if working_dir_path.is_relative() {
                        let resolved = path.parent().unwrap_or(search_path).join(working_dir_path);
                        manifest.working_dir = Some(resolved.to_string_lossy().into_owned());
                    }
                }
                if let Some(executable) = manifest.executable.clone() {
                    let executable_path = PathBuf::from(&executable);
                    if executable_path.is_relative() && executable_path.components().count() > 1 {
                        let resolved = path.parent().unwrap_or(search_path).join(executable_path);
                        manifest.executable = Some(resolved.to_string_lossy().into_owned());
                    }
                }
                manifests.push(manifest);
            }
        }
        // Keep discovery deterministic so capability conflicts always resolve against the same
        // plugin order regardless of filesystem enumeration order.
        manifests.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.version.cmp(&right.version))
                .then_with(|| left.executable.cmp(&right.executable))
        });
        Ok(manifests)
    }

    pub async fn start_process(&self, manifest: &PluginManifest) -> Result<PluginProcess> {
        PluginProcess::start(manifest).await
    }

    pub async fn start(
        &self,
        manifest: &PluginManifest,
        local_peer: PeerDescriptor,
        local_initialize: Option<InitializeMessage>,
    ) -> Result<Supervisor> {
        let process = self.start_process(manifest).await?;
        Supervisor::from_process(process, local_peer, local_initialize).await
    }
}
