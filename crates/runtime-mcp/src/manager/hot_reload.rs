//! # MCP 配置热加载
//!
//! 使用 `notify` crate 监听 `.mcp.json` 和 settings 文件变更，
//! 通过 `tokio::sync::mpsc` 通道发送变更事件，避免文件监听线程触碰异步状态。

use std::path::{Path, PathBuf};

use log::{info, warn};
use notify::{Config as NotifyConfig, Event, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

/// 配置变更事件。
#[derive(Debug, Clone)]
pub enum McpConfigChangeEvent {
    /// `.mcp.json` 文件变更。
    McpJsonChanged,
    /// settings 文件变更（审批状态等）。
    SettingsChanged,
}

/// MCP 配置热加载监听器。
///
/// 启动后台文件监听，检测到变更后通过 mpsc 通道发送事件。
pub struct McpHotReload {
    /// 文件监听器（持有以保持活跃）。
    _watcher: RecommendedWatcher,
    /// 事件接收端（由调用方持有并消费）。
    rx: mpsc::UnboundedReceiver<McpConfigChangeEvent>,
}

impl McpHotReload {
    /// 创建并启动配置热加载监听。
    ///
    /// 监听指定目录下的 `.mcp.json` 和 settings 文件变更。
    /// 返回 `(McpHotReload, mpsc::UnboundedSender)` 以便调用方接收事件。
    pub fn new(mcp_json_path: &Path, settings_path: Option<&Path>) -> Self {
        let mut watched_paths = vec![mcp_json_path.to_path_buf()];
        if let Some(settings_path) = settings_path {
            watched_paths.push(settings_path.to_path_buf());
        }
        Self::new_with_paths(watched_paths)
    }

    /// 创建并启动多路径热加载监听。
    ///
    /// 所有给定路径共用一个 watcher，事件按“目标文件是否命中”收敛为同一条
    /// reload 信号，便于 runtime 统一重建 MCP surface。
    pub fn new_with_paths(watched_paths: Vec<PathBuf>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        let watcher = Self::create_watcher(tx, watched_paths);

        Self {
            _watcher: watcher,
            rx,
        }
    }

    /// 获取事件接收端的可变引用。
    ///
    /// 调用方在异步循环中 `rx.recv().await` 接收变更事件。
    pub fn events(&mut self) -> &mut mpsc::UnboundedReceiver<McpConfigChangeEvent> {
        &mut self.rx
    }

    /// 创建文件监听器。
    fn create_watcher(
        tx: mpsc::UnboundedSender<McpConfigChangeEvent>,
        watched_paths: Vec<PathBuf>,
    ) -> RecommendedWatcher {
        let target_paths = watched_paths
            .into_iter()
            .collect::<std::collections::HashSet<_>>();

        let target_paths_for_events = target_paths.clone();
        let watcher = RecommendedWatcher::new(
            move |result: std::result::Result<Event, notify::Error>| match result {
                Ok(event) => {
                    let is_mcp_json = event.paths.iter().any(|path| {
                        path.file_name().is_some_and(|name| name == ".mcp.json")
                            && target_paths_for_events.contains(path)
                    });
                    let is_settings = event.paths.iter().any(|path| {
                        target_paths_for_events.contains(path)
                            && path.file_name().is_some_and(|name| name != ".mcp.json")
                    });

                    if is_mcp_json {
                        let _ = tx.send(McpConfigChangeEvent::McpJsonChanged);
                    }
                    if is_settings {
                        let _ = tx.send(McpConfigChangeEvent::SettingsChanged);
                    }
                },
                Err(e) => {
                    warn!("MCP hot reload file watch error: {}", e);
                },
            },
            NotifyConfig::default(),
        );

        match watcher {
            Ok(mut w) => {
                let parent_dirs = target_paths
                    .iter()
                    .filter_map(|path| path.parent().map(Path::to_path_buf))
                    .collect::<std::collections::HashSet<_>>();
                for parent in parent_dirs {
                    if !parent.exists() {
                        continue;
                    }
                    if let Err(error) = w.watch(&parent, RecursiveMode::NonRecursive) {
                        warn!(
                            "MCP hot reload: failed to watch directory '{}': {}",
                            parent.display(),
                            error
                        );
                    } else {
                        info!("MCP hot reload: watching directory '{}'", parent.display());
                    }
                }

                w
            },
            Err(e) => {
                warn!("MCP hot reload: failed to create watcher: {}", e);
                // 返回一个空的 watcher（不可用但不会 panic）
                // 这里用 panic 代替不合理，因为 RecommendedWatcher::new 可能返回 Err
                // 但构造函数不应该 panic——后续 start_hot_reload 会处理
                panic!("MCP hot reload: failed to create file watcher: {}", e);
            },
        }
    }
}

/// 计算 .mcp.json 文件的路径。
///
/// 按优先级查找：当前目录 → 项目根目录。
pub fn resolve_mcp_json_path(project_dir: &Path) -> PathBuf {
    let candidates = [project_dir.join(".mcp.json")];

    for candidate in &candidates {
        if candidate.exists() {
            return candidate.clone();
        }
    }

    // 即使文件不存在也返回默认路径（监听目录等待创建）
    candidates[0].clone()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn test_resolve_mcp_json_path() {
        let dir = std::env::temp_dir().join("mcp_hot_reload_test_resolve");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let path = resolve_mcp_json_path(&dir);
        assert_eq!(path, dir.join(".mcp.json"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_resolve_existing_mcp_json() {
        let dir = std::env::temp_dir().join("mcp_hot_reload_test_existing");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(".mcp.json"), "{}").unwrap();

        let path = resolve_mcp_json_path(&dir);
        assert!(path.exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_event_debug_format() {
        let event = McpConfigChangeEvent::McpJsonChanged;
        assert!(format!("{:?}", event).contains("McpJsonChanged"));

        let event = McpConfigChangeEvent::SettingsChanged;
        assert!(format!("{:?}", event).contains("SettingsChanged"));
    }
}
