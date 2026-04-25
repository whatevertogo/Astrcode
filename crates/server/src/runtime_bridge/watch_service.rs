//! server-owned 文件变更监听 contract。
//!
//! watch source / event / port / service 类型不经由 `application` 暴露。

use std::sync::Arc;

use tokio::sync::broadcast;

use crate::application_error_bridge::ServerRouteError;

const WATCH_EVENT_CAPACITY: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum WatchSource {
    GlobalAgentDefinitions,
    AgentDefinitions { working_dir: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WatchEvent {
    pub source: WatchSource,
    pub affected_paths: Vec<String>,
}

pub(crate) trait WatchPort: Send + Sync {
    fn start_watch(
        &self,
        sources: Vec<WatchSource>,
        tx: broadcast::Sender<WatchEvent>,
    ) -> Result<(), ServerRouteError>;

    fn stop_all(&self) -> Result<(), ServerRouteError>;

    fn add_source(&self, source: WatchSource) -> Result<(), ServerRouteError>;

    fn remove_source(&self, source: &WatchSource) -> Result<(), ServerRouteError>;
}

pub(crate) struct WatchService {
    port: Arc<dyn WatchPort>,
    tx: broadcast::Sender<WatchEvent>,
}

impl WatchService {
    pub(crate) fn new(port: Arc<dyn WatchPort>) -> Self {
        let (tx, _) = broadcast::channel(WATCH_EVENT_CAPACITY);
        Self { port, tx }
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<WatchEvent> {
        self.tx.subscribe()
    }

    pub(crate) fn start_watch(&self, sources: Vec<WatchSource>) -> Result<(), ServerRouteError> {
        self.port.start_watch(sources, self.tx.clone())
    }

    pub(crate) fn stop_all(&self) -> Result<(), ServerRouteError> {
        self.port.stop_all()
    }

    pub(crate) fn add_source(&self, source: WatchSource) -> Result<(), ServerRouteError> {
        self.port.add_source(source)
    }

    pub(crate) fn remove_source(&self, source: &WatchSource) -> Result<(), ServerRouteError> {
        self.port.remove_source(source)
    }
}

impl std::fmt::Debug for WatchService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WatchService").finish_non_exhaustive()
    }
}
