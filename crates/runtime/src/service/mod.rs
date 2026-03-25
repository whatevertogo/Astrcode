use std::sync::Arc;

use async_trait::async_trait;
use dashmap::DashMap;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use crate::agent_loop::AgentLoop;
use crate::config::load_config;
use crate::provider_factory::ConfigFileProviderFactory;
use astrcode_core::{AstrError, CapabilityRouter, RuntimeHandle};

#[cfg(test)]
mod baselines;
mod config_ops;
mod observability;
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
use observability::RuntimeObservability;
pub use observability::{
    OperationMetricsSnapshot, ReplayMetricsSnapshot, ReplayPath, RuntimeObservabilitySnapshot,
};

pub struct RuntimeService {
    sessions: DashMap<String, Arc<SessionState>>,
    loop_: RwLock<Arc<AgentLoop>>,
    config: Mutex<crate::config::Config>,
    session_load_lock: Mutex<()>,
    observability: Arc<RuntimeObservability>,
    /// Token used to signal server shutdown
    shutdown_token: CancellationToken,
}

impl RuntimeService {
    pub fn from_capabilities(capabilities: CapabilityRouter) -> ServiceResult<Self> {
        let config = load_config().map_err(ServiceError::from)?;
        let loop_ = AgentLoop::from_capabilities(Arc::new(ConfigFileProviderFactory), capabilities);
        Ok(Self {
            sessions: DashMap::new(),
            loop_: RwLock::new(Arc::new(loop_)),
            config: Mutex::new(config),
            session_load_lock: Mutex::new(()),
            observability: Arc::new(RuntimeObservability::default()),
            shutdown_token: CancellationToken::new(),
        })
    }

    pub async fn current_loop(&self) -> Arc<AgentLoop> {
        self.loop_.read().await.clone()
    }

    pub async fn replace_capabilities(&self, capabilities: CapabilityRouter) -> ServiceResult<()> {
        let next_loop = Arc::new(AgentLoop::from_capabilities(
            Arc::new(ConfigFileProviderFactory),
            capabilities,
        ));
        *self.loop_.write().await = next_loop;
        Ok(())
    }

    pub fn loaded_session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn running_session_ids(&self) -> Vec<String> {
        let mut running_sessions = self
            .sessions
            .iter()
            .filter(|entry| {
                entry
                    .value()
                    .running
                    .load(std::sync::atomic::Ordering::SeqCst)
            })
            .map(|entry| entry.key().clone())
            .collect::<Vec<_>>();
        running_sessions.sort();
        running_sessions
    }

    pub fn observability_snapshot(&self) -> RuntimeObservabilitySnapshot {
        self.observability.snapshot()
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

#[async_trait]
impl RuntimeHandle for RuntimeService {
    fn runtime_name(&self) -> &'static str {
        "astrcode-runtime"
    }

    fn runtime_kind(&self) -> &'static str {
        "native"
    }

    async fn shutdown(&self, timeout_secs: u64) -> std::result::Result<(), AstrError> {
        RuntimeService::shutdown(self, timeout_secs).await;
        Ok(())
    }
}
