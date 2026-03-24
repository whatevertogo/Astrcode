use std::path::PathBuf;

use astrcode_core::{PluginManifest, Result};
use astrcode_protocol::plugin::{InitializeMessage, PeerDescriptor};

use crate::{PluginProcess, Supervisor};

#[derive(Debug, Clone)]
pub struct PluginInstance {
    pub manifest: PluginManifest,
}

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

            for entry in std::fs::read_dir(search_path).map_err(|error| {
                astrcode_core::AstrError::io(
                    format!(
                        "failed to read plugin directory '{}'",
                        search_path.display()
                    ),
                    error,
                )
            })? {
                let entry = entry.map_err(|error| {
                    astrcode_core::AstrError::io(
                        format!(
                            "failed to inspect plugin entry in '{}'",
                            search_path.display()
                        ),
                        error,
                    )
                })?;
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
                    continue;
                }
                let raw = std::fs::read_to_string(&path).map_err(|error| {
                    astrcode_core::AstrError::io(
                        format!("failed to read plugin manifest '{}'", path.display()),
                        error,
                    )
                })?;
                let mut manifest = PluginManifest::from_toml(&raw)?;
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
        Ok(manifests)
    }

    pub fn load(&self, manifest: &PluginManifest) -> Result<PluginInstance> {
        Ok(PluginInstance {
            manifest: manifest.clone(),
        })
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
