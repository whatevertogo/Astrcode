//! 文件变更监听用例。
//!
//! 负责：
//! - **订阅**：注册对配置文件、agent 定义文件的变更兴趣
//! - **监听**：通过端口接收底层文件系统事件
//! - **推送**：将变更事件广播给订阅者（config 热重载、agent 热更新等）
//!
//! IO 和文件系统轮询通过 `WatchPort` 端口委托给适配器层。

use std::sync::Arc;

use tokio::sync::broadcast;

use crate::ApplicationError;

// ============================================================
// 业务模型
// ============================================================

/// 变更事件的来源。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WatchSource {
    /// 全局配置文件变更（`~/.astrcode/config.json`）。
    GlobalConfig,
    /// 全局 agent 定义目录变更（`~/.claude/agents` / `~/.astrcode/agents`）。
    GlobalAgentDefinitions,
    /// 项目级配置覆盖变更（`<project>/.astrcode/config.json`）。
    ProjectConfig { working_dir: String },
    /// Agent 定义文件变更（`<project>/.astrcode/agents/`）。
    AgentDefinitions { working_dir: String },
}

/// 文件变更通知。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchEvent {
    /// 变更来源。
    pub source: WatchSource,
    /// 受影响的文件路径（相对于项目根目录）。
    pub affected_paths: Vec<String>,
}

// ============================================================
// Watch 用例端口
// ============================================================

/// 文件系统监听端口，由适配器层实现。
///
/// 适配器负责：
/// - 实际的文件系统监听（inotify、FSEvents、ReadDirectoryChangesW）
/// - 防抖（合并短时间内的多次变更）
/// - 动态添加/移除监听路径
pub trait WatchPort: Send + Sync {
    /// 启动对指定来源的监听，变更事件发送到 tx。
    fn start_watch(
        &self,
        sources: Vec<WatchSource>,
        tx: broadcast::Sender<WatchEvent>,
    ) -> Result<(), ApplicationError>;

    /// 停止所有监听。
    fn stop_all(&self) -> Result<(), ApplicationError>;

    /// 动态添加新的监听来源。
    fn add_source(&self, source: WatchSource) -> Result<(), ApplicationError>;

    /// 移除指定来源的监听。
    fn remove_source(&self, source: &WatchSource) -> Result<(), ApplicationError>;
}

// ============================================================
// Watch 用例服务
// ============================================================

const WATCH_EVENT_CAPACITY: usize = 256;

/// 文件变更监听用例服务。
///
/// 通过 `WatchPort` 订阅文件变更，通过 broadcast channel 推送给订阅者。
pub struct WatchService {
    port: Arc<dyn WatchPort>,
    tx: broadcast::Sender<WatchEvent>,
}

impl WatchService {
    pub fn new(port: Arc<dyn WatchPort>) -> Self {
        let (tx, _) = broadcast::channel(WATCH_EVENT_CAPACITY);
        Self { port, tx }
    }

    /// 用例：订阅变更通知。
    ///
    /// 返回一个 broadcast receiver，调用方通过 `.recv()` 接收推送。
    pub fn subscribe(&self) -> broadcast::Receiver<WatchEvent> {
        self.tx.subscribe()
    }

    /// 用例：启动监听指定来源的文件变更。
    pub fn start_watch(&self, sources: Vec<WatchSource>) -> Result<(), ApplicationError> {
        self.port.start_watch(sources, self.tx.clone())
    }

    /// 用例：停止所有监听。
    pub fn stop_all(&self) -> Result<(), ApplicationError> {
        self.port.stop_all()
    }

    /// 用例：动态添加新的监听来源。
    pub fn add_source(&self, source: WatchSource) -> Result<(), ApplicationError> {
        self.port.add_source(source)
    }

    /// 用例：移除指定来源的监听。
    pub fn remove_source(&self, source: &WatchSource) -> Result<(), ApplicationError> {
        self.port.remove_source(source)
    }
}

impl std::fmt::Debug for WatchService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WatchService").finish_non_exhaustive()
    }
}
