use std::{
    any::Any,
    collections::HashMap,
    sync::{Arc, Mutex},
};

use astrcode_agent_runtime::HookDispatcher;
use astrcode_core::{AgentLifecycleStatus, AgentProfile, CapabilityInvoker, Result};
use astrcode_host_session::{
    HookDispatch as HostSessionHookDispatch, SessionCatalog, SubRunHandle,
};
use astrcode_llm_contract::LlmProvider;

use crate::{
    SessionInfoProvider,
    agent_control_bridge::{ServerAgentControlPort, ServerLiveSubRunStatus},
    ports::{AgentKernelPort, AgentSessionPort, AppSessionPort, ServerKernelControlError},
};

#[path = "session_runtime_owner_bridge_impl.rs"]
mod implementation;

pub(crate) use implementation::bootstrap_session_runtime;

#[derive(Default)]
pub(crate) struct ActiveSessionRegistry {
    counts: Mutex<HashMap<String, usize>>,
}

impl ActiveSessionRegistry {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn mark_running(&self, session_id: &str) {
        let mut counts = self
            .counts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *counts.entry(session_id.to_string()).or_default() += 1;
    }

    pub(crate) fn mark_idle(&self, session_id: &str) {
        let mut counts = self
            .counts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(count) = counts.get_mut(session_id) else {
            return;
        };
        if *count <= 1 {
            counts.remove(session_id);
        } else {
            *count -= 1;
        }
    }

    pub(crate) fn running_session_ids(&self) -> Vec<String> {
        let mut session_ids = self
            .counts
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        session_ids.sort();
        session_ids
    }
}

pub(crate) trait ServerCapabilitySurfacePort: Send + Sync {
    fn replace_capability_invokers(&self, invokers: Vec<Arc<dyn CapabilityInvoker>>) -> Result<()>;
}

#[allow(dead_code)]
#[async_trait::async_trait]
pub(crate) trait ServerRuntimeTestSupport: Send + Sync {
    fn list_running_session_ids(&self) -> Vec<String>;

    async fn append_event(
        &self,
        session_id: &str,
        event: astrcode_core::StorageEvent,
    ) -> Result<()>;

    async fn prepare_test_turn_runtime(&self, session_id: &str, turn_id: &str) -> Result<u64>;

    async fn complete_test_turn_runtime(&self, session_id: &str, generation: u64) -> Result<()>;

    async fn replay_stored_events(
        &self,
        session_id: &str,
    ) -> Result<Vec<astrcode_core::StoredEvent>>;

    async fn register_root_agent(
        &self,
        agent_id: String,
        session_id: String,
        profile_id: String,
    ) -> std::result::Result<SubRunHandle, ServerKernelControlError>;

    async fn spawn_independent_child(
        &self,
        profile: &AgentProfile,
        session_id: String,
        child_session_id: String,
        parent_turn_id: String,
        parent_agent_id: String,
    ) -> std::result::Result<SubRunHandle, ServerKernelControlError>;

    async fn set_lifecycle(
        &self,
        sub_run_or_agent_id: &str,
        new_status: AgentLifecycleStatus,
    ) -> Option<()>;

    async fn pending_parent_delivery_count(&self, parent_session_id: &str) -> usize;

    async fn query_root_status(&self, session_id: &str) -> Option<ServerLiveSubRunStatus>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerAgentControlLimits {
    pub max_depth: usize,
    pub max_concurrent: usize,
    pub finalized_retain_limit: usize,
    pub inbox_capacity: usize,
    pub parent_delivery_capacity: usize,
}

pub(crate) struct ServerSessionRuntimeBootstrapInput {
    pub capability_invokers: Vec<Arc<dyn CapabilityInvoker>>,
    pub llm_provider: Arc<dyn LlmProvider>,
    pub session_catalog: Arc<SessionCatalog>,
    pub mode_catalog: Arc<crate::mode_catalog_service::ServerModeCatalog>,
    pub agent_limits: ServerAgentControlLimits,
    pub hook_dispatcher: Option<Arc<dyn HookDispatcher>>,
    pub owner_hook_dispatcher: Option<Arc<dyn HostSessionHookDispatch>>,
    pub hook_snapshot_id: Arc<dyn Fn() -> String + Send + Sync>,
}

pub(crate) struct ServerBootstrappedSessionRuntime {
    pub app_sessions: Arc<dyn AppSessionPort>,
    pub agent_sessions: Arc<dyn AgentSessionPort>,
    pub agent_kernel: Arc<dyn AgentKernelPort>,
    pub agent_control: Arc<dyn ServerAgentControlPort>,
    pub capability_surface: Arc<dyn ServerCapabilitySurfacePort>,
    pub sessions: Arc<dyn SessionInfoProvider>,
    pub keepalive: Arc<dyn Any + Send + Sync>,
    pub test_support: Arc<dyn ServerRuntimeTestSupport>,
}
