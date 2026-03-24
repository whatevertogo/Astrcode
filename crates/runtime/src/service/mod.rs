use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::agent_loop::AgentLoop;
use crate::config::load_config;
use crate::provider_factory::ConfigFileProviderFactory;
use astrcode_core::CapabilityRouter;

mod config_ops;
mod replay;
mod session_ops;
mod session_state;
mod support;
mod turn_ops;
mod types;

use self::session_state::SessionState;
pub use self::types::{
    PromptAccepted, ServiceError, ServiceResult, SessionEventRecord, SessionMessage, SessionReplay,
    SessionReplaySource,
};

pub struct RuntimeService {
    sessions: DashMap<String, Arc<SessionState>>,
    loop_: Arc<AgentLoop>,
    config: Mutex<crate::config::Config>,
    session_load_lock: Mutex<()>,
    /// Token used to signal server shutdown
    shutdown_token: CancellationToken,
}

impl RuntimeService {
    pub fn from_capabilities(capabilities: CapabilityRouter) -> ServiceResult<Self> {
        let config = load_config().map_err(ServiceError::from)?;
        let loop_ = AgentLoop::from_capabilities(Arc::new(ConfigFileProviderFactory), capabilities);
        Ok(Self {
            sessions: DashMap::new(),
            loop_: Arc::new(loop_),
            config: Mutex::new(config),
            session_load_lock: Mutex::new(()),
            shutdown_token: CancellationToken::new(),
        })
    }

    /// Returns a clone of the shutdown token for use in handlers
    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown_token.clone()
    }

    /// Initiates graceful shutdown:
    /// 1. Signals all running turns to cancel
    /// 2. Waits for all turns to complete (with timeout)
    /// 3. Returns when all sessions are idle or timeout elapsed
    pub async fn shutdown(&self, timeout_secs: u64) {
        log::info!("Initiating graceful shutdown...");

        // Signal shutdown to all handlers
        self.shutdown_token.cancel();

        // Cancel all running sessions
        for entry in self.sessions.iter() {
            let session = entry.value();
            if session.running.load(std::sync::atomic::Ordering::SeqCst) {
                if let Ok(cancel) = session.cancel.lock().map(|g| g.clone()) {
                    cancel.cancel();
                }
            }
        }

        // Wait for all sessions to become idle
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(timeout_secs);

        loop {
            let running_count = self
                .sessions
                .iter()
                .filter(|entry| {
                    entry
                        .value()
                        .running
                        .load(std::sync::atomic::Ordering::SeqCst)
                })
                .count();

            if running_count == 0 {
                log::info!("All sessions are idle, shutdown complete");
                return;
            }

            if start.elapsed() >= timeout {
                log::warn!(
                    "Shutdown timeout elapsed, {} sessions still running",
                    running_count
                );
                return;
            }

            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}
