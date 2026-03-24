use std::path::PathBuf;

use astrcode_core::{PluginManifest, Result};

use crate::PluginProcess;

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
                manifests.push(PluginManifest::from_toml(&raw)?);
            }
        }
        Ok(manifests)
    }

    pub fn load(&self, manifest: &PluginManifest) -> Result<PluginInstance> {
        Ok(PluginInstance {
            manifest: manifest.clone(),
        })
    }

    pub async fn start(&self, manifest: &PluginManifest) -> Result<PluginProcess> {
        PluginProcess::start(manifest).await
    }
}
