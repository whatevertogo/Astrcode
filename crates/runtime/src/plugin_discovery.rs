//! # 插件发现 (Plugin Discovery)
//!
//! 从环境变量配置的路径中扫描并加载插件清单。
//!
//! ## 配置来源
//!
//! 插件搜索路径来自 `ASTRCODE_PLUGIN_DIRS` 环境变量，
//! 多个路径使用路径分隔符分隔（Windows 为 `;`，Unix 为 `:`）。
//! 若环境变量未设置，返回空列表（不阻塞运行时启动）。

use std::path::PathBuf;

use astrcode_core::{env::ASTRCODE_PLUGIN_DIRS_ENV, AstrError, PluginManifest};
use astrcode_plugin::PluginLoader;

/// 从环境变量读取插件搜索路径。
///
/// 插件搜索路径来自 `ASTRCODE_PLUGIN_DIRS` 环境变量，
/// 多个路径使用路径分隔符分隔（Windows 为 `;`，Unix 为 `:`）。
/// 若环境变量未设置，返回空列表（不阻塞运行时启动）。
pub(crate) fn configured_plugin_paths() -> Vec<PathBuf> {
    match std::env::var_os(ASTRCODE_PLUGIN_DIRS_ENV) {
        Some(raw_paths) => std::env::split_paths(&raw_paths).collect(),
        None => Vec::new(),
    }
}

/// 从给定的搜索路径中扫描并加载所有插件清单。
///
/// 若搜索路径为空，直接返回空列表（不报错），
/// 这样运行时可以在没有插件配置的情况下正常启动。
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
