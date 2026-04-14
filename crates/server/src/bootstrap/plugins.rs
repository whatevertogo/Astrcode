//! # 插件发现、装载与物化
//!
//! 独立于主组合根的插件装配模块，负责：
//! - 发现 `search_paths` 中的 `.toml` 插件清单
//! - 启动插件进程并完成握手
//! - 将插件能力物化为 `CapabilityInvoker` 列表
//! - 更新 `PluginRegistry` 的生命周期状态
//!
//! 组合根通过 `bootstrap_plugins` 获取物化结果，
//! 不需要了解 loader/supervisor/peer 的内部细节。

use std::{
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use astrcode_adapter_skills::{SkillSource, SkillSpec, collect_asset_files, is_valid_skill_name};
use astrcode_plugin::{PluginLoader, Supervisor, default_initialize_message, default_profiles};
use astrcode_protocol::plugin::{PeerDescriptor, SkillDescriptor};
use log::warn;

#[cfg(test)]
use super::deps::core::home::resolve_home_dir;
use super::deps::core::{CapabilityInvoker, PluginRegistry};

/// 插件装配结果。
pub(crate) struct PluginBootstrapResult {
    /// 物化后的插件能力调用器。
    pub invokers: Vec<Arc<dyn CapabilityInvoker>>,
    /// 物化后的插件 skill。
    pub skills: Vec<SkillSpec>,
    /// 插件注册表引用（治理视图使用）。
    pub registry: Arc<PluginRegistry>,
    /// 活跃的插件 supervisor 列表（shutdown 时需要关闭）。
    pub supervisors: Vec<Arc<Supervisor>>,
    /// 插件搜索路径。
    pub search_paths: Vec<PathBuf>,
}

/// 发现、装载并物化所有插件。
///
/// 流程：
/// 1. 从 search_paths 发现 .toml 插件清单
/// 2. 逐个启动插件进程并完成握手
/// 3. 从握手结果中提取 CapabilityInvoker
/// 4. 更新 PluginRegistry 状态
///
/// 容错：单个插件失败不影响其他插件，失败信息记录到 registry。
#[cfg(test)]
pub(crate) async fn bootstrap_plugins(search_paths: Vec<PathBuf>) -> PluginBootstrapResult {
    let skill_root = resolve_default_plugin_skill_root();
    bootstrap_plugins_with_skill_root(search_paths, skill_root).await
}

pub(crate) async fn bootstrap_plugins_with_skill_root(
    search_paths: Vec<PathBuf>,
    plugin_skill_root: PathBuf,
) -> PluginBootstrapResult {
    let registry = Arc::new(PluginRegistry::default());
    let loader = PluginLoader {
        search_paths: search_paths.clone(),
    };

    let manifests = match loader.discover() {
        Ok(manifests) => manifests,
        Err(error) => {
            log::warn!("plugin discovery failed: {error}");
            return PluginBootstrapResult {
                invokers: Vec::new(),
                skills: Vec::new(),
                registry,
                supervisors: Vec::new(),
                search_paths,
            };
        },
    };

    log::info!("discovered {} plugin(s)", manifests.len());

    let local_peer = PeerDescriptor {
        id: "astrcode-host".to_string(),
        name: "Astrcode Host".to_string(),
        role: astrcode_protocol::plugin::PeerRole::Core,
        version: env!("CARGO_PKG_VERSION").to_string(),
        supported_profiles: vec!["coding".to_string()],
        metadata: serde_json::json!({}),
    };
    let init_message =
        default_initialize_message(local_peer.clone(), Vec::new(), default_profiles());

    let mut all_invokers: Vec<Arc<dyn CapabilityInvoker>> = Vec::new();
    let mut all_skills = Vec::new();
    let mut supervisors = Vec::new();

    for manifest in manifests {
        let name = manifest.name.clone();
        log::info!("loading plugin '{name}'...");

        registry.record_discovered(manifest.clone());

        match loader
            .start(&manifest, local_peer.clone(), Some(init_message.clone()))
            .await
        {
            Ok(supervisor) => {
                let supervisor = Arc::new(supervisor);
                // 物化能力
                let invokers = supervisor.capability_invokers();
                let capabilities: Vec<_> = invokers
                    .iter()
                    .map(|invoker| invoker.capability_spec())
                    .collect();
                let (skills, warnings) = materialize_plugin_skills(
                    plugin_skill_root.as_path(),
                    &name,
                    supervisor.declared_skills(),
                );

                log::info!(
                    "plugin '{name}' initialized with {} capabilities and {} skills",
                    capabilities.len(),
                    skills.len()
                );

                registry.record_initialized(manifest, capabilities, warnings);
                all_invokers.extend(invokers);
                all_skills.extend(skills);
                supervisors.push(supervisor);
            },
            Err(error) => {
                log::error!("plugin '{name}' failed to initialize: {error}");
                registry.record_failed(
                    manifest,
                    error.to_string(),
                    Vec::new(),
                    vec![format!("initialization failed: {error}")],
                );
            },
        }
    }

    PluginBootstrapResult {
        invokers: all_invokers,
        skills: all_skills,
        registry,
        supervisors,
        search_paths,
    }
}

fn materialize_plugin_skills(
    plugin_skill_root: &Path,
    plugin_name: &str,
    skill_descriptors: Vec<SkillDescriptor>,
) -> (Vec<SkillSpec>, Vec<String>) {
    let mut skills = Vec::new();
    let mut warnings = Vec::new();

    for descriptor in skill_descriptors {
        if !is_valid_skill_name(&descriptor.name) {
            warnings.push(format!(
                "plugin '{}' declared invalid skill name '{}'; expected kebab-case",
                plugin_name, descriptor.name
            ));
            continue;
        }

        let (skill_root, asset_files, materialize_warning) =
            materialize_plugin_skill_assets(plugin_skill_root, plugin_name, &descriptor);
        if let Some(warning) = materialize_warning {
            warnings.push(warning);
        }

        skills.push(SkillSpec {
            id: descriptor.name.clone(),
            name: descriptor.name,
            description: descriptor.description,
            guide: descriptor.guide,
            skill_root,
            asset_files,
            allowed_tools: descriptor.allowed_tools,
            source: SkillSource::Plugin,
        });
    }

    (skills, warnings)
}

fn materialize_plugin_skill_assets(
    plugin_skill_root: &Path,
    plugin_name: &str,
    descriptor: &SkillDescriptor,
) -> (Option<String>, Vec<String>, Option<String>) {
    materialize_plugin_skill_assets_under_root(plugin_skill_root, plugin_name, descriptor)
}

fn materialize_plugin_skill_assets_under_root(
    plugin_skill_root: &Path,
    plugin_name: &str,
    descriptor: &SkillDescriptor,
) -> (Option<String>, Vec<String>, Option<String>) {
    let skill_root = plugin_skill_root
        .join(sanitize_path_segment(plugin_name))
        .join(&descriptor.name);

    if let Err(error) = fs::create_dir_all(&skill_root) {
        return (
            None,
            Vec::new(),
            Some(format!(
                "plugin '{}' skill '{}' could not create asset directory '{}': {}",
                plugin_name,
                descriptor.name,
                skill_root.display(),
                error
            )),
        );
    }

    let skill_markdown = format!(
        "---\nname: {}\ndescription: {}\n---\n\n{}\n",
        descriptor.name, descriptor.description, descriptor.guide
    );
    let skill_markdown_path = skill_root.join("SKILL.md");
    if let Err(error) = write_asset_if_changed(&skill_markdown_path, &skill_markdown) {
        return (
            None,
            Vec::new(),
            Some(format!(
                "plugin '{}' skill '{}' could not materialize SKILL.md: {}",
                plugin_name, descriptor.name, error
            )),
        );
    }

    for asset in &descriptor.assets {
        if !is_safe_relative_asset_path(&asset.relative_path) {
            return (
                None,
                Vec::new(),
                Some(format!(
                    "plugin '{}' skill '{}' contains unsafe asset path '{}'",
                    plugin_name, descriptor.name, asset.relative_path
                )),
            );
        }

        let asset_path = skill_root.join(
            asset
                .relative_path
                .replace('/', std::path::MAIN_SEPARATOR_STR),
        );
        if let Some(parent) = asset_path.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                return (
                    None,
                    Vec::new(),
                    Some(format!(
                        "plugin '{}' skill '{}' could not create asset directory '{}': {}",
                        plugin_name,
                        descriptor.name,
                        parent.display(),
                        error
                    )),
                );
            }
        }

        if !asset.encoding.eq_ignore_ascii_case("utf-8") {
            warn!(
                "plugin '{}' skill '{}' asset '{}' uses unsupported encoding '{}'; storing as raw \
                 text",
                plugin_name, descriptor.name, asset.relative_path, asset.encoding
            );
        }

        if let Err(error) = write_asset_if_changed(&asset_path, &asset.content) {
            return (
                None,
                Vec::new(),
                Some(format!(
                    "plugin '{}' skill '{}' could not materialize asset '{}': {}",
                    plugin_name, descriptor.name, asset.relative_path, error
                )),
            );
        }
    }

    (
        Some(skill_root.to_string_lossy().into_owned()),
        collect_asset_files(&skill_root),
        None,
    )
}

#[cfg(test)]
fn resolve_default_plugin_skill_root() -> PathBuf {
    match resolve_home_dir() {
        Ok(home_dir) => home_dir
            .join(".astrcode")
            .join("runtime")
            .join("plugin-skills"),
        Err(_) => PathBuf::from(".astrcode")
            .join("runtime")
            .join("plugin-skills"),
    }
}

fn sanitize_path_segment(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches(['-', '.', ' '])
        .to_string();

    if sanitized.is_empty() {
        "plugin".to_string()
    } else {
        sanitized
    }
}

fn is_safe_relative_asset_path(relative_path: &str) -> bool {
    let path = Path::new(relative_path);
    !path.is_absolute()
        && path.components().all(|component| {
            matches!(component, Component::Normal(_)) || matches!(component, Component::CurDir)
        })
}

fn write_asset_if_changed(path: &Path, content: &str) -> std::io::Result<()> {
    if fs::read_to_string(path).ok().as_deref() == Some(content) {
        return Ok(());
    }

    fs::write(path, content)
}

#[cfg(test)]
mod tests {
    use astrcode_protocol::plugin::{SkillAssetDescriptor, SkillDescriptor};

    use super::*;
    use crate::bootstrap::deps::core::plugin::{PluginHealth, PluginState};

    #[tokio::test]
    async fn bootstrap_with_empty_paths_returns_empty() {
        let result = bootstrap_plugins(vec![]).await;
        assert!(result.invokers.is_empty());
        assert!(result.skills.is_empty());
        assert!(result.supervisors.is_empty());
        assert!(result.registry.snapshot().is_empty());
    }

    #[tokio::test]
    async fn bootstrap_with_nonexistent_path_returns_empty() {
        let result = bootstrap_plugins(vec![PathBuf::from("/nonexistent/path")]).await;
        assert!(result.invokers.is_empty());
        assert!(result.skills.is_empty());
        assert!(result.supervisors.is_empty());
    }

    #[tokio::test]
    async fn plugin_failure_is_recorded_in_registry() {
        // 创建一个包含无效 .toml 的临时目录
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");
        let plugin_toml = temp_dir.path().join("broken.toml");
        std::fs::write(
            &plugin_toml,
            r#"
name = "broken-plugin"
version = "0.1.0"
description = "A broken plugin"
plugin_type = ["Tool"]
capabilities = []
executable = "nonexistent-binary"
"#,
        )
        .expect("toml should be written");

        let result = bootstrap_plugins(vec![temp_dir.path().to_path_buf()]).await;

        // 插件被发现了，但启动失败（进程不存在）
        assert!(result.supervisors.is_empty(), "不应有成功的 supervisor");
        let entries = result.registry.snapshot();
        assert_eq!(entries.len(), 1, "应有一个 registry 条目");

        let entry = &entries[0];
        assert_eq!(entry.manifest.name, "broken-plugin");
        // 插件发现成功但初始化失败
        assert!(
            matches!(entry.state, PluginState::Failed),
            "应为 Failed 状态: {:?}",
            entry.state
        );
        assert!(entry.failure.is_some(), "失败信息不应被静默吞掉");
        assert_eq!(entry.health, PluginHealth::Unavailable);
    }

    #[tokio::test]
    async fn multiple_plugins_partial_failure() {
        let temp_dir = tempfile::tempdir().expect("tempdir should be created");

        // 第一个无效插件
        std::fs::write(
            temp_dir.path().join("a-broken.toml"),
            r#"
name = "a-broken"
version = "0.1.0"
description = "Broken"
plugin_type = ["Tool"]
capabilities = []
executable = "no-such-binary"
"#,
        )
        .expect("toml should be written");

        // 第二个也是无效的（不同的名字）
        std::fs::write(
            temp_dir.path().join("b-broken.toml"),
            r#"
name = "b-broken"
version = "0.1.0"
description = "Also broken"
plugin_type = ["Tool"]
capabilities = []
executable = "also-missing"
"#,
        )
        .expect("toml should be written");

        let result = bootstrap_plugins(vec![temp_dir.path().to_path_buf()]).await;

        // 两个都失败
        let entries = result.registry.snapshot();
        assert_eq!(entries.len(), 2, "两个插件都应有 registry 条目");

        for entry in &entries {
            assert!(
                matches!(entry.state, PluginState::Failed),
                "{} 应为 Failed: {:?}",
                entry.manifest.name,
                entry.state
            );
            assert!(
                entry.failure.is_some(),
                "{} 的失败信息不应被静默吞掉",
                entry.manifest.name
            );
        }
    }

    #[test]
    fn plugin_declared_skills_materialize_into_skill_specs() {
        let temp_home = tempfile::tempdir().expect("temp home should be created");
        let plugin_skill_root = temp_home
            .path()
            .join(".astrcode")
            .join("runtime")
            .join("plugin-skills");
        let descriptor = SkillDescriptor {
            name: "repo-search".to_string(),
            description: "Search the repo".to_string(),
            guide: "Use references under ${ASTRCODE_SKILL_DIR}.".to_string(),
            allowed_tools: vec!["grep".to_string()],
            assets: vec![SkillAssetDescriptor {
                relative_path: "references/api.md".to_string(),
                content: "# API".to_string(),
                encoding: "utf-8".to_string(),
            }],
            metadata: serde_json::Value::Null,
        };
        let (skill_root, asset_files, warning) = materialize_plugin_skill_assets_under_root(
            &plugin_skill_root,
            "demo-plugin",
            &descriptor,
        );
        let (skills, warnings) =
            materialize_plugin_skills(&plugin_skill_root, "demo-plugin", vec![descriptor]);

        assert!(warning.is_none(), "direct materialization should not warn");
        assert!(
            warnings.is_empty(),
            "plugin skill materialization should not warn"
        );
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].source, SkillSource::Plugin);
        assert_eq!(asset_files, vec!["references/api.md".to_string()]);
        let skill_root = skill_root.expect("plugin skill root should be materialized");
        assert!(
            Path::new(&skill_root)
                .join("references")
                .join("api.md")
                .is_file()
        );
    }
}
