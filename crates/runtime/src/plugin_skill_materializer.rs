//! 插件 skill 物化器。
//!
//! 将插件声明的 skill 校验、落盘和清理逻辑从 runtime surface 组装流程中拆出，
//! 让 `runtime_surface_assembler` 只负责 orchestration，而不是同时关心文件系统细节。

use std::{
    collections::HashSet,
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use astrcode_core::{AstrError, ManagedRuntimeComponent, PluginManifest, project::astrcode_dir};
use astrcode_protocol::plugin::SkillDescriptor;
use astrcode_runtime_skill_loader::{
    SKILL_FILE_NAME, SkillSource, SkillSpec, collect_asset_files, is_valid_skill_name,
    merge_skill_layers,
};
use async_trait::async_trait;

pub(crate) struct MaterializedPluginSkills {
    pub(crate) skills: Vec<SkillSpec>,
    pub(crate) warnings: Vec<String>,
    pub(crate) managed_components: Vec<Arc<dyn ManagedRuntimeComponent>>,
}

pub(crate) fn materialize_plugin_skills(
    manifest: &PluginManifest,
    surface_materialization_id: &str,
    descriptors: &[SkillDescriptor],
    available_tool_names: &HashSet<String>,
) -> MaterializedPluginSkills {
    let mut warnings = Vec::new();
    let mut skills = Vec::new();
    let mut materialized_roots = Vec::new();

    for descriptor in descriptors {
        match materialize_plugin_skill(
            manifest,
            surface_materialization_id,
            descriptor,
            available_tool_names,
        ) {
            Ok(Some((skill, root, skill_warnings))) => {
                // 插件内部同名 skill 也遵循统一覆盖规则，避免 surface assembler
                // 额外维护一套去重语义。
                skills = merge_skill_layers(skills, vec![skill]);
                materialized_roots.push(root);
                warnings.extend(skill_warnings);
            },
            Ok(None) => {},
            Err(warning) => {
                log::warn!("plugin '{}' skill rejected: {}", manifest.name, warning);
                warnings.push(warning);
            },
        }
    }

    let managed_components = materialized_roots
        .into_iter()
        .map(|root_dir| {
            Arc::new(MaterializedSkillAssetsComponent { root_dir })
                as Arc<dyn ManagedRuntimeComponent>
        })
        .collect();

    MaterializedPluginSkills {
        skills,
        warnings,
        managed_components,
    }
}

fn materialize_plugin_skill(
    manifest: &PluginManifest,
    surface_materialization_id: &str,
    descriptor: &SkillDescriptor,
    available_tool_names: &HashSet<String>,
) -> std::result::Result<Option<(SkillSpec, PathBuf, Vec<String>)>, String> {
    let skill_name = descriptor.name.trim();
    if !is_valid_skill_name(skill_name) {
        return Err(format!(
            "skill '{}' from plugin '{}' has invalid name; expected kebab-case",
            descriptor.name, manifest.name
        ));
    }
    if descriptor.description.trim().is_empty() {
        return Err(format!(
            "skill '{}' from plugin '{}' is missing description",
            descriptor.name, manifest.name
        ));
    }
    if descriptor.guide.trim().is_empty() {
        return Err(format!(
            "skill '{}' from plugin '{}' is missing guide markdown",
            descriptor.name, manifest.name
        ));
    }

    let skill_root = astrcode_dir()
        .map_err(|error| {
            format!(
                "failed to resolve Astrcode home for plugin '{}' skill '{}': {}",
                manifest.name, descriptor.name, error
            )
        })?
        .join("runtime")
        .join("plugin-skills")
        .join(&manifest.name)
        .join(surface_materialization_id)
        .join(skill_name);

    if let Some(parent) = skill_root.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "failed to create skill cache parent '{}' for plugin '{}' skill '{}': {}",
                parent.display(),
                manifest.name,
                descriptor.name,
                error
            )
        })?;
    }

    fs::create_dir_all(&skill_root).map_err(|error| {
        format!(
            "failed to create skill cache '{}' for plugin '{}' skill '{}': {}",
            skill_root.display(),
            manifest.name,
            descriptor.name,
            error
        )
    })?;

    let skill_markdown = render_materialized_skill_markdown(descriptor);
    write_asset_if_changed(&skill_root.join(SKILL_FILE_NAME), &skill_markdown).map_err(
        |error| {
            format!(
                "failed to materialize guide for plugin '{}' skill '{}': {}",
                manifest.name, descriptor.name, error
            )
        },
    )?;

    for asset in &descriptor.assets {
        if !is_safe_relative_asset_path(&asset.relative_path) {
            return Err(format!(
                "skill '{}' from plugin '{}' has unsafe asset path '{}'",
                descriptor.name, manifest.name, asset.relative_path
            ));
        }
        if !asset.encoding.eq_ignore_ascii_case("utf-8") {
            return Err(format!(
                "skill '{}' from plugin '{}' uses unsupported asset encoding '{}'",
                descriptor.name, manifest.name, asset.encoding
            ));
        }

        let asset_path = skill_root.join(
            asset
                .relative_path
                .replace('/', std::path::MAIN_SEPARATOR_STR),
        );
        if let Some(parent) = asset_path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create asset directory '{}' for plugin '{}' skill '{}': {}",
                    parent.display(),
                    manifest.name,
                    descriptor.name,
                    error
                )
            })?;
        }

        write_asset_if_changed(&asset_path, &asset.content).map_err(|error| {
            format!(
                "failed to materialize asset '{}' for plugin '{}' skill '{}': {}",
                asset.relative_path, manifest.name, descriptor.name, error
            )
        })?;
    }

    let mut allowed_tools = Vec::new();
    let mut warnings = Vec::new();
    for tool_name in &descriptor.allowed_tools {
        if available_tool_names.contains(tool_name) {
            allowed_tools.push(tool_name.clone());
        } else {
            log::warn!(
                "plugin '{}' skill '{}' dropped unknown allowed tool '{}'",
                manifest.name,
                descriptor.name,
                tool_name
            );
            warnings.push(format!(
                "skill '{}' dropped unknown allowed tool '{}'",
                descriptor.name, tool_name
            ));
        }
    }

    Ok(Some((
        SkillSpec {
            id: skill_name.to_string(),
            name: skill_name.to_string(),
            description: descriptor.description.trim().to_string(),
            guide: descriptor.guide.trim().to_string(),
            skill_root: Some(skill_root.to_string_lossy().into_owned()),
            asset_files: collect_asset_files(&skill_root),
            allowed_tools,
            source: SkillSource::Plugin,
        },
        skill_root,
        warnings,
    )))
}

fn render_materialized_skill_markdown(descriptor: &SkillDescriptor) -> String {
    format!(
        "---\nname: {}\ndescription: {}\n---\n{}\n",
        descriptor.name.trim(),
        descriptor.description.trim(),
        descriptor.guide.trim()
    )
}

fn is_safe_relative_asset_path(relative_path: &str) -> bool {
    let path = Path::new(relative_path);
    !path.is_absolute()
        && path.components().all(|component| {
            matches!(component, Component::Normal(_)) || matches!(component, Component::CurDir)
        })
}

fn write_asset_if_changed(path: &Path, content: &str) -> std::io::Result<()> {
    // 故意忽略：读取失败表示文件不存在或不可读，需要重写
    if fs::read_to_string(path).ok().as_deref() == Some(content) {
        return Ok(());
    }

    fs::write(path, content)
}

struct MaterializedSkillAssetsComponent {
    root_dir: PathBuf,
}

#[async_trait]
impl ManagedRuntimeComponent for MaterializedSkillAssetsComponent {
    fn component_name(&self) -> String {
        format!("plugin-skill-assets:{}", self.root_dir.display())
    }

    async fn shutdown_component(&self) -> astrcode_core::Result<()> {
        if !self.root_dir.exists() {
            return Ok(());
        }
        fs::remove_dir_all(&self.root_dir).map_err(|error| {
            AstrError::io(
                format!(
                    "failed to remove materialized plugin skill cache '{}'",
                    self.root_dir.display()
                ),
                error,
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use astrcode_core::{PluginType, test_support::TestEnvGuard};
    use serde_json::json;

    use super::*;

    fn manifest(name: &str) -> PluginManifest {
        PluginManifest {
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

    #[test]
    fn materialize_plugin_skills_filters_unknown_allowed_tools_into_warnings() {
        let _guard = TestEnvGuard::new();
        let result = materialize_plugin_skills(
            &manifest("demo"),
            "surface-1",
            &[SkillDescriptor {
                name: "repo-search".to_string(),
                description: "Search the repository".to_string(),
                guide: "# Guide\nUse ripgrep.".to_string(),
                allowed_tools: vec!["shell".to_string(), "missing.tool".to_string()],
                assets: vec![astrcode_protocol::plugin::SkillAssetDescriptor {
                    relative_path: "references/usage.md".to_string(),
                    content: "usage".to_string(),
                    encoding: "utf-8".to_string(),
                }],
                metadata: json!({}),
            }],
            &HashSet::from(["shell".to_string(), "Skill".to_string()]),
        );

        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].allowed_tools, vec!["shell".to_string()]);
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("missing.tool"))
        );
        assert!(result.skills[0].skill_root.is_some());
        assert!(
            result.skills[0]
                .asset_files
                .iter()
                .any(|path| path == "references/usage.md")
        );
    }

    #[test]
    fn materialize_plugin_skills_drops_invalid_descriptors_without_blocking_valid_ones() {
        let _guard = TestEnvGuard::new();
        let result = materialize_plugin_skills(
            &manifest("demo"),
            "surface-2",
            &[
                SkillDescriptor {
                    name: "valid-skill".to_string(),
                    description: "A valid skill".to_string(),
                    guide: "# Guide\nDo the thing.".to_string(),
                    allowed_tools: vec![],
                    assets: vec![],
                    metadata: json!({}),
                },
                SkillDescriptor {
                    name: "broken-skill".to_string(),
                    description: "Broken".to_string(),
                    guide: "# Guide\nBroken".to_string(),
                    allowed_tools: vec![],
                    assets: vec![astrcode_protocol::plugin::SkillAssetDescriptor {
                        relative_path: "../escape.txt".to_string(),
                        content: "escape".to_string(),
                        encoding: "utf-8".to_string(),
                    }],
                    metadata: json!({}),
                },
            ],
            &HashSet::new(),
        );

        assert_eq!(result.skills.len(), 1);
        assert_eq!(result.skills[0].id, "valid-skill");
        assert_eq!(result.managed_components.len(), 1);
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("unsafe asset path"));
    }
}
