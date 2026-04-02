//! # 插件系统
//!
//! 定义了插件的元数据（清单）、注册表和生命周期状态。
//!
//! ## 模块说明
//!
//! - `manifest`: 插件清单（`PluginManifest`）和类型（`PluginType`）
//! - `registry`: 插件注册表（`PluginRegistry`），管理插件状态和健康检查

mod manifest;
mod registry;

pub use manifest::{PluginManifest, PluginType};
pub use registry::{PluginEntry, PluginHealth, PluginRegistry, PluginState};
