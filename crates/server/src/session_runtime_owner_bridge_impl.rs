use std::sync::Arc;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
use astrcode_core::{
    AgentLifecycleStatus, AgentProfile, EventTranslator, SessionId, SessionTurnAcquireResult,
    SessionTurnLease, StorageEvent, StoredEvent,
};
use astrcode_core::{CapabilityInvoker, Result};
use astrcode_host_session::SessionCatalog;

#[cfg(test)]
use crate::session_runtime_owner_bridge::ServerRuntimeTestSupport;
use crate::{
    SessionInfoProvider,
    agent_control_bridge::ServerAgentControlPort,
    agent_control_registry::{AgentControlLimits, AgentControlRegistry},
    capability_router::CapabilityRouter,
    ports::{
        AgentKernelPort, AgentSessionPort, AppSessionPort,
        kernel_bridge::build_server_kernel_bridge, session_bridge::build_server_session_bridge,
    },
    session_runtime_owner_bridge::{
        ActiveSessionRegistry, ServerBootstrappedSessionRuntime, ServerCapabilitySurfacePort,
        ServerSessionRuntimeBootstrapInput,
    },
    session_runtime_port::adapter::build_session_runtime_port,
};
#[cfg(test)]
use crate::{
    agent_control_bridge::ServerLiveSubRunStatus, ports::ServerKernelControlError,
    session_runtime_port::SessionRuntimePort,
};

pub(crate) fn bootstrap_session_runtime(
    input: ServerSessionRuntimeBootstrapInput,
) -> Result<ServerBootstrappedSessionRuntime> {
    let capability_router = CapabilityRouter::builder()
        .build()
        .expect("empty capability router should build");
    capability_router.replace_invokers(input.capability_invokers)?;
    let agent_control_registry = Arc::new(AgentControlRegistry::from_limits(AgentControlLimits {
        max_depth: input.agent_limits.max_depth,
        max_concurrent: input.agent_limits.max_concurrent,
        finalized_retain_limit: input.agent_limits.finalized_retain_limit,
        inbox_capacity: input.agent_limits.inbox_capacity,
        parent_delivery_capacity: input.agent_limits.parent_delivery_capacity,
    }));
    let active_sessions = Arc::new(ActiveSessionRegistry::new());
    let session_runtime = build_session_runtime_port(
        Arc::clone(&input.session_catalog),
        Arc::clone(&input.mode_catalog),
        Arc::clone(&agent_control_registry),
        capability_router.clone(),
        Arc::clone(&input.llm_provider),
        Arc::clone(&active_sessions),
    );
    let session_bridge =
        build_server_session_bridge(Arc::clone(&input.session_catalog), session_runtime.clone());
    let kernel_bridge = build_server_kernel_bridge(session_runtime.clone());
    let app_sessions: Arc<dyn AppSessionPort> = session_bridge.clone();
    let agent_sessions: Arc<dyn AgentSessionPort> = session_bridge;
    let agent_kernel: Arc<dyn AgentKernelPort> = kernel_bridge.clone();
    let agent_control: Arc<dyn ServerAgentControlPort> = kernel_bridge;
    let capability_surface: Arc<dyn ServerCapabilitySurfacePort> =
        Arc::new(CapabilitySurfaceBridge {
            capability_router: capability_router.clone(),
        });

    Ok(ServerBootstrappedSessionRuntime {
        app_sessions,
        agent_sessions,
        agent_kernel,
        agent_control,
        capability_surface,
        sessions: Arc::new(SessionRuntimeInfoBridge {
            session_catalog: Arc::clone(&input.session_catalog),
            active_sessions: Arc::clone(&active_sessions),
        }),
        keepalive: Arc::new(SessionRuntimeCompatKeepalive {
            _agent_control: agent_control_registry,
        }),
        #[cfg(test)]
        test_support: Arc::new(SessionRuntimeTestSupportBridge {
            session_catalog: input.session_catalog,
            active_sessions,
            session_runtime: session_runtime.clone(),
            next_generation: AtomicU64::new(1),
            prepared_turns: std::sync::Mutex::new(std::collections::HashMap::new()),
        }),
    })
}

struct CapabilitySurfaceBridge {
    capability_router: CapabilityRouter,
}

impl ServerCapabilitySurfacePort for CapabilitySurfaceBridge {
    fn replace_capability_invokers(&self, invokers: Vec<Arc<dyn CapabilityInvoker>>) -> Result<()> {
        self.capability_router.replace_invokers(invokers)
    }
}

struct SessionRuntimeCompatKeepalive {
    _agent_control: Arc<AgentControlRegistry>,
}

struct SessionRuntimeInfoBridge {
    session_catalog: Arc<SessionCatalog>,
    active_sessions: Arc<ActiveSessionRegistry>,
}

impl SessionInfoProvider for SessionRuntimeInfoBridge {
    fn loaded_session_count(&self) -> usize {
        self.session_catalog.list_loaded_sessions().len()
    }

    fn running_session_ids(&self) -> Vec<String> {
        self.active_sessions.running_session_ids()
    }
}

#[cfg(test)]
struct PreparedTestTurn {
    session_id: String,
    _lease: Box<dyn SessionTurnLease>,
}

#[cfg(test)]
struct SessionRuntimeTestSupportBridge {
    session_catalog: Arc<SessionCatalog>,
    active_sessions: Arc<ActiveSessionRegistry>,
    session_runtime: Arc<dyn SessionRuntimePort>,
    next_generation: AtomicU64,
    prepared_turns: std::sync::Mutex<std::collections::HashMap<u64, PreparedTestTurn>>,
}

#[cfg(test)]
#[async_trait::async_trait]
impl ServerRuntimeTestSupport for SessionRuntimeTestSupportBridge {
    fn list_running_session_ids(&self) -> Vec<String> {
        self.active_sessions.running_session_ids()
    }

    async fn append_event(&self, session_id: &str, event: StorageEvent) -> Result<()> {
        let state = self
            .session_catalog
            .session_state(&SessionId::from(session_id.to_string()))
            .await?;
        let mut translator = EventTranslator::new(state.current_phase()?);
        let stored = state.writer.clone().append(event).await?;
        let records = state.translate_store_and_cache(&stored, &mut translator)?;
        for record in records {
            let _ = state.broadcaster.send(record);
        }
        Ok(())
    }

    async fn prepare_test_turn_runtime(&self, session_id: &str, turn_id: &str) -> Result<u64> {
        let session_id_value = SessionId::from(session_id.to_string());
        let acquire = self
            .session_catalog
            .try_acquire_turn(&session_id_value, turn_id)
            .await?;
        let SessionTurnAcquireResult::Acquired(lease) = acquire else {
            return Err(astrcode_core::AstrError::Validation(format!(
                "session '{}' already has an active turn lease",
                session_id
            )));
        };
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        self.prepared_turns
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                generation,
                PreparedTestTurn {
                    session_id: session_id.to_string(),
                    _lease: lease,
                },
            );
        self.active_sessions.mark_running(session_id);
        Ok(generation)
    }

    async fn complete_test_turn_runtime(&self, _session_id: &str, generation: u64) -> Result<()> {
        let prepared = self
            .prepared_turns
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&generation);
        if let Some(prepared) = prepared {
            self.active_sessions.mark_idle(&prepared.session_id);
        }
        Ok(())
    }

    async fn replay_stored_events(&self, session_id: &str) -> Result<Vec<StoredEvent>> {
        self.session_catalog
            .replay_stored_events(&SessionId::from(session_id.to_string()))
            .await
    }

    async fn register_root_agent(
        &self,
        agent_id: String,
        session_id: String,
        profile_id: String,
    ) -> std::result::Result<astrcode_host_session::SubRunHandle, ServerKernelControlError> {
        self.session_runtime
            .register_root_agent(agent_id, session_id, profile_id)
            .await
    }

    async fn spawn_independent_child(
        &self,
        profile: &AgentProfile,
        session_id: String,
        child_session_id: String,
        parent_turn_id: String,
        parent_agent_id: String,
    ) -> std::result::Result<astrcode_host_session::SubRunHandle, ServerKernelControlError> {
        self.session_runtime
            .spawn_independent_child(
                profile,
                session_id,
                child_session_id,
                parent_turn_id,
                parent_agent_id,
            )
            .await
    }

    async fn set_lifecycle(
        &self,
        sub_run_or_agent_id: &str,
        new_status: AgentLifecycleStatus,
    ) -> Option<()> {
        self.session_runtime
            .set_lifecycle(sub_run_or_agent_id, new_status)
            .await
    }

    async fn pending_parent_delivery_count(&self, parent_session_id: &str) -> usize {
        self.session_runtime
            .pending_parent_delivery_count(parent_session_id)
            .await
    }

    async fn query_root_status(&self, session_id: &str) -> Option<ServerLiveSubRunStatus> {
        self.session_runtime.query_root_status(session_id).await
    }
}
