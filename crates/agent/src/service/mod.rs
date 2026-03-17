use std::sync::Arc;

use dashmap::DashMap;
use tokio::sync::Mutex;

use crate::agent_loop::AgentLoop;
use crate::config::load_config;
use crate::provider_factory::ConfigFileProviderFactory;
use crate::tool_registry::ToolRegistry;

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

pub struct AgentService {
    sessions: DashMap<String, Arc<SessionState>>,
    loop_: Arc<AgentLoop>,
    config: Mutex<crate::config::Config>,
    session_load_lock: Mutex<()>,
}

impl AgentService {
    pub fn new(registry: ToolRegistry) -> ServiceResult<Self> {
        let config = load_config().map_err(ServiceError::from)?;
        let loop_ = AgentLoop::new(Arc::new(ConfigFileProviderFactory), registry);
        Ok(Self {
            sessions: DashMap::new(),
            loop_: Arc::new(loop_),
            config: Mutex::new(config),
            session_load_lock: Mutex::new(()),
        })
    }
}
