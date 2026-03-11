mod ipc;
mod model_service;
mod presentation;
mod prompt_service;
mod session_service;
mod support;

use std::collections::HashMap;

use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use astrcode_core::AgentRuntime;

pub use self::presentation::{ConfigView, CurrentModelInfo, ModelOption, SessionMessage};

use self::support::sync_runtime_working_dir;

pub struct AgentHandle {
    runtime: Mutex<AgentRuntime>,
    cancel: Mutex<Option<CancellationToken>>,
    reasoning_cache: Mutex<HashMap<String, HashMap<usize, String>>>,
    session_id: Mutex<String>,
}

impl AgentHandle {
    pub fn new() -> anyhow::Result<Self> {
        let runtime = match AgentRuntime::resume_last()? {
            Some(runtime) => {
                sync_runtime_working_dir(&runtime);
                runtime
            }
            None => AgentRuntime::new_session()?,
        };

        let session_id = runtime.session_id.clone();
        let mut reasoning_cache = HashMap::new();
        reasoning_cache.insert(session_id.clone(), runtime.reasoning_cache_snapshot());
        Ok(Self {
            runtime: Mutex::new(runtime),
            cancel: Mutex::new(None),
            reasoning_cache: Mutex::new(reasoning_cache),
            session_id: Mutex::new(session_id),
        })
    }
}

#[cfg(test)]
mod tests;
