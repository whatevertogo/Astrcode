//! 插件加载器—— 发现、解析和启动插件。
//!
//! 本模块负责：
//!
//! - **发现**: 在配置的搜索路径中扫描 `.toml` 插件清单文件
//! - **解析**: 解析 `PluginManifest` 并处理相对路径
//! - **启动**: 启动插件进程并完成握手
//!
//! ## 插件发现流程
//!
//! 1. 遍历所有 `search_paths`
//! 2. 读取目录中的 `.toml` 文件
//! 3. 解析为 `PluginManifest`
//! 4. 将相对路径（`working_dir`、`executable`）解析为绝对路径
//! 5. 按名称、版本、可执行文件路径排序以保证确定性

use std::path::PathBuf;

use astrcode_core::{AstrError, PluginManifest, Result};
use astrcode_protocol::plugin::{InitializeMessage, PeerDescriptor};

use crate::{PluginProcess, Supervisor};

pub fn parse_plugin_manifest_toml(raw: &str) -> Result<PluginManifest> {
    toml::from_str(raw).map_err(|error| {
        AstrError::Validation(format!("failed to parse plugin manifest TOML: {error}"))
    })
}

/// 插件加载器。
///
/// 维护插件搜索路径列表，提供发现、解析和启动插件的功能。
///
/// # 搜索路径
///
/// 每个搜索路径是一个目录，加载器会扫描其中的 `.toml` 文件作为插件清单。
/// 路径不存在或无法读取时会记录警告并跳过，不会导致整体失败。
#[derive(Debug, Default, Clone)]
pub struct PluginLoader {
    pub search_paths: Vec<PathBuf>,
}

/// Resolve a relative path field in place.
///
/// If `require_components_gt_1` is true, only resolve paths that contain
/// directory separators (e.g. `./bin/plugin`), avoiding bare executable names.
fn resolve_relative_path(
    path_field: &mut Option<String>,
    manifest_path: &std::path::Path,
    search_path: &std::path::Path,
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

impl PluginLoader {
    /// 在所有搜索路径中发现插件清单。
    ///
    /// # 容错设计
    ///
    /// 此方法采用"尽力而为"策略：
    /// - 目录不存在或无法读取 → 记录警告，跳过
    /// - 单个条目无法检查 → 记录警告，跳过
    /// - 文件不是 `.toml` → 静默跳过
    /// - 清单解析失败 → 记录警告，跳过
    ///
    /// 这样确保单个插件的问题不会影响其他插件的加载。
    ///
    /// # 路径解析
    ///
    /// 清单中的 `working_dir` 和 `executable` 如果是相对路径，
    /// 会相对于清单文件所在目录进行解析。
    /// `executable` 只有在包含路径分隔符（`components().count() > 1`）时才解析，
    /// 这是为了避免将简单的可执行文件名（如 `my-plugin`）错误地解析为相对路径。
    ///
    /// # 排序
    ///
    /// 返回结果按名称、版本、可执行文件路径排序，
    /// 确保能力冲突解析的确定性，不受文件系统枚举顺序影响。
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

    /// 启动插件进程但不进行握手。
    ///
    /// 仅创建子进程和传输层，不调用 `initialize()`。
    pub async fn start_process(&self, manifest: &PluginManifest) -> Result<PluginProcess> {
        PluginProcess::start(manifest).await
    }

    /// 启动插件进程并完成握手。
    ///
    /// 这是加载插件的完整流程：启动进程 → 创建 Peer → 发送 InitializeMessage →
    /// 等待 InitializeResultData → 返回 Supervisor。
    ///
    /// # 参数
    ///
    /// * `manifest` - 插件清单
    /// * `local_peer` - 本地（宿主）的 peer 描述
    /// * `local_initialize` - 可选的自定义初始化消息；为 `None` 时使用默认值
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
