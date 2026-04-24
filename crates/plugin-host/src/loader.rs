use std::path::{Path, PathBuf};

use astrcode_core::{AstrError, Result};

use crate::{
    CommandManifestEntry, PluginDescriptor, PluginManifest, PromptManifestEntry,
    ProviderManifestEntry, ResourceManifestEntry, SkillManifestEntry, ThemeManifestEntry,
    descriptor::{
        CommandDescriptor, PluginSourceKind, PromptDescriptor, ProviderDescriptor,
        ResourceDescriptor, SkillDescriptor, ThemeDescriptor,
    },
};

pub fn parse_plugin_manifest_toml(raw: &str) -> Result<PluginManifest> {
    toml::from_str(raw).map_err(|error| {
        AstrError::Validation(format!("failed to parse plugin manifest TOML: {error}"))
    })
}

#[derive(Debug, Default, Clone)]
pub struct PluginLoader {
    pub search_paths: Vec<PathBuf>,
}

fn resolve_relative_path(
    path_field: &mut Option<String>,
    manifest_path: &Path,
    search_path: &Path,
    require_components_gt_1: bool,
) {
    let Some(value) = path_field.clone() else {
        return;
    };
    let path = PathBuf::from(&value);
    if !path.is_relative() {
        return;
    }
    if require_components_gt_1 && path.components().count() <= 1 {
        return;
    }
    let resolved = manifest_path.parent().unwrap_or(search_path).join(path);
    *path_field = Some(resolved.to_string_lossy().into_owned());
}

fn manifest_to_descriptor(manifest: PluginManifest, manifest_path: &Path) -> PluginDescriptor {
    let source_ref = manifest
        .executable
        .clone()
        .unwrap_or_else(|| manifest_path.to_string_lossy().into_owned());
    PluginDescriptor {
        plugin_id: manifest.name.clone(),
        display_name: manifest.name,
        version: manifest.version,
        source_kind: PluginSourceKind::Process,
        source_ref,
        enabled: true,
        priority: 0,
        launch_command: manifest.executable,
        launch_args: manifest.args,
        working_dir: manifest.working_dir,
        repository: manifest.repository,
        tools: manifest.capabilities,
        hooks: Vec::new(),
        providers: manifest
            .providers
            .into_iter()
            .map(
                |ProviderManifestEntry { id, api_kind }| ProviderDescriptor {
                    provider_id: id,
                    api_kind,
                },
            )
            .collect(),
        resources: manifest
            .resources
            .into_iter()
            .map(
                |ResourceManifestEntry { id, kind, locator }| ResourceDescriptor {
                    resource_id: id,
                    kind,
                    locator,
                },
            )
            .collect(),
        commands: manifest
            .commands
            .into_iter()
            .map(|CommandManifestEntry { id, entry_ref }| CommandDescriptor {
                command_id: id,
                entry_ref,
            })
            .collect(),
        themes: manifest
            .themes
            .into_iter()
            .map(|ThemeManifestEntry { id }| ThemeDescriptor { theme_id: id })
            .collect(),
        prompts: manifest
            .prompts
            .into_iter()
            .map(|PromptManifestEntry { id, body }| PromptDescriptor {
                prompt_id: id,
                body,
            })
            .collect(),
        skills: manifest
            .skills
            .into_iter()
            .map(|SkillManifestEntry { id, entry_ref }| SkillDescriptor {
                skill_id: id,
                entry_ref,
            })
            .collect(),
        modes: Vec::new(),
    }
}

impl PluginLoader {
    pub fn discover_descriptors(&self) -> Result<Vec<PluginDescriptor>> {
        let mut descriptors = Vec::new();
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
                },
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
                    },
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
                    },
                };
                let mut manifest = match parse_plugin_manifest_toml(&raw) {
                    Ok(manifest) => manifest,
                    Err(error) => {
                        log::warn!(
                            "skipping plugin manifest '{}' because it could not be parsed: {}",
                            path.display(),
                            error
                        );
                        continue;
                    },
                };
                resolve_relative_path(&mut manifest.working_dir, &path, search_path, false);
                resolve_relative_path(&mut manifest.executable, &path, search_path, true);
                descriptors.push(manifest_to_descriptor(manifest, &path));
            }
        }

        descriptors.sort_by(|left, right| {
            left.plugin_id
                .cmp(&right.plugin_id)
                .then_with(|| left.version.cmp(&right.version))
                .then_with(|| left.launch_command.cmp(&right.launch_command))
        });
        Ok(descriptors)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{PluginLoader, parse_plugin_manifest_toml};

    fn unique_temp_dir(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic enough")
            .as_nanos();
        std::env::temp_dir().join(format!("astrcode-plugin-host-{name}-{suffix}"))
    }

    #[test]
    fn parse_plugin_manifest_toml_reads_manifest() {
        let manifest = parse_plugin_manifest_toml(
            r#"
name = "repo-inspector"
version = "0.1.0"
description = "inspect repo"
plugin_type = ["Tool"]
capabilities = []
executable = "./bin/repo-inspector"
args = ["--stdio"]
working_dir = "."
repository = "https://example.com/repo-inspector"
"#,
        )
        .expect("manifest should parse");

        assert_eq!(manifest.name, "repo-inspector");
        assert_eq!(manifest.args, vec!["--stdio".to_string()]);
        assert_eq!(manifest.working_dir.as_deref(), Some("."));
    }

    #[test]
    fn parse_plugin_manifest_toml_reads_provider_contributions() {
        let manifest = parse_plugin_manifest_toml(
            r#"
name = "corp-provider"
version = "0.1.0"
description = "corp provider"
plugin_type = ["Provider"]
capabilities = []
executable = "./bin/corp-provider"
repository = "https://example.com/corp-provider"

[[providers]]
id = "corp-ai"
api_kind = "openai-compatible"
"#,
        )
        .expect("manifest should parse");

        assert_eq!(manifest.providers.len(), 1);
        assert_eq!(manifest.providers[0].id, "corp-ai");
        assert_eq!(manifest.providers[0].api_kind, "openai-compatible");
    }

    #[test]
    fn discover_descriptors_resolves_relative_paths_and_sorts() {
        let root = unique_temp_dir("discover");
        fs::create_dir_all(root.join("alpha")).expect("temp dir should create");
        fs::create_dir_all(root.join("beta")).expect("temp dir should create");
        fs::write(
            root.join("beta").join("beta.toml"),
            r#"
name = "beta"
version = "0.1.0"
description = "beta"
plugin_type = ["Tool"]
capabilities = []
executable = "./bin/beta"
args = ["--serve"]
working_dir = "."
repository = "https://example.com/beta"
"#,
        )
        .expect("beta manifest should write");
        fs::write(
            root.join("alpha").join("alpha.toml"),
            r#"
name = "alpha"
version = "0.1.0"
description = "alpha"
plugin_type = ["Tool"]
capabilities = []
executable = "./bin/alpha"
args = []
working_dir = "."
repository = "https://example.com/alpha"
"#,
        )
        .expect("alpha manifest should write");

        let loader = PluginLoader {
            search_paths: vec![root.join("alpha"), root.join("beta")],
        };
        let descriptors = loader
            .discover_descriptors()
            .expect("descriptors should be discovered");

        assert_eq!(
            descriptors
                .iter()
                .map(|descriptor| descriptor.plugin_id.as_str())
                .collect::<Vec<_>>(),
            vec!["alpha", "beta"]
        );
        assert!(
            descriptors[0]
                .launch_command
                .as_deref()
                .expect("launch command should resolve")
                .contains("bin")
        );
        assert!(
            descriptors[0]
                .working_dir
                .as_deref()
                .expect("working dir should resolve")
                .contains("alpha")
        );

        let _ = fs::remove_dir_all(root);
    }
}
