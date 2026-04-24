use std::time::{SystemTime, UNIX_EPOCH};

use astrcode_core::{CapabilityContext, Result};
use astrcode_protocol::plugin::{
    CapabilityWireDescriptor, InvocationContext, PeerDescriptor, WorkspaceRef,
};
use serde_json::Value;

use crate::{
    PluginActiveSnapshot, PluginDescriptor, PluginInitializeState, PluginLoader, PluginRegistry,
    ResourceCatalog,
    backend::{
        BuiltinPluginRuntimeHandle, ExternalPluginRuntimeHandle, PluginBackendHealthReport,
        PluginBackendKind, PluginBackendPlan,
    },
    default_local_peer_descriptor, resources_discover,
};

/// `plugin-host` 的最小外观。
///
/// 它先只承接 registry 的 staging / commit / rollback，
/// 后续再把 loader、backend 和资源发现逐步接进来。
#[derive(Debug)]
pub struct PluginHost {
    registry: PluginRegistry,
    local_peer: PeerDescriptor,
}

#[derive(Debug)]
pub struct PluginHostReload {
    pub descriptors: Vec<PluginDescriptor>,
    pub snapshot: PluginActiveSnapshot,
    pub builtin_backends: Vec<BuiltinPluginRuntimeHandle>,
    pub external_backends: Vec<ExternalPluginRuntimeHandle>,
    pub resources: ResourceCatalog,
    pub backend_health: ExternalBackendHealthCatalog,
    pub negotiated_plugins: NegotiatedPluginCatalog,
    pub runtime_catalog: ActivePluginRuntimeCatalog,
}

#[path = "host_dispatch.rs"]
mod dispatch;

pub use dispatch::*;

#[path = "host_catalog.rs"]
mod catalog;

pub use catalog::*;


#[path = "host_reload.rs"]
mod reload;

impl PluginHost {
    pub fn new() -> Self {
        Self::with_local_peer(default_local_peer_descriptor())
    }

    pub fn with_local_peer(local_peer: PeerDescriptor) -> Self {
        Self {
            registry: PluginRegistry::default(),
            local_peer,
        }
    }

    pub fn registry(&self) -> &PluginRegistry {
        &self.registry
    }

    pub fn local_peer(&self) -> &PeerDescriptor {
        &self.local_peer
    }

    pub fn stage_candidate(
        &self,
        descriptors: impl IntoIterator<Item = PluginDescriptor>,
    ) -> Result<PluginActiveSnapshot> {
        self.registry.stage_candidate(descriptors)
    }

    pub fn commit_candidate(&self) -> Option<PluginActiveSnapshot> {
        self.registry.commit_candidate()
    }

    pub fn rollback_candidate(&self) -> Option<PluginActiveSnapshot> {
        self.registry.rollback_candidate()
    }

    pub fn active_snapshot(&self) -> Option<PluginActiveSnapshot> {
        self.registry.active_snapshot()
    }

    pub fn backend_plans(
        &self,
        descriptors: &[PluginDescriptor],
    ) -> Result<Vec<PluginBackendPlan>> {
        descriptors
            .iter()
            .map(PluginBackendPlan::from_descriptor)
            .collect()
    }

    fn sort_descriptors_for_reload(descriptors: &mut [PluginDescriptor]) {
        descriptors.sort_by(|left, right| {
            left.plugin_id
                .cmp(&right.plugin_id)
                .then_with(|| left.version.cmp(&right.version))
                .then_with(|| left.source_ref.cmp(&right.source_ref))
        });
    }

    pub async fn start_external_process_backends(
        &self,
        plans: &[PluginBackendPlan],
    ) -> Result<Vec<ExternalPluginRuntimeHandle>> {
        self.start_external_process_backends_with_capabilities(plans, &[])
            .await
    }

    pub async fn start_external_process_backends_with_capabilities(
        &self,
        plans: &[PluginBackendPlan],
        capabilities: &[CapabilityWireDescriptor],
    ) -> Result<Vec<ExternalPluginRuntimeHandle>> {
        let mut backends = Vec::new();
        for plan in plans {
            match plan.backend_kind {
                PluginBackendKind::Process | PluginBackendKind::Command => {
                    let backend = plan.start_process().await?;
                    let initialize_state = PluginInitializeState::with_defaults(
                        self.local_peer.clone(),
                        capabilities.to_vec(),
                    );
                    backends.push(
                        ExternalPluginRuntimeHandle::from_backend(backend)
                            .with_initialize_state(initialize_state),
                    );
                },
                PluginBackendKind::InProcess | PluginBackendKind::Http => {
                    // builtin 和 http backend 在后续阶段走各自 owner 路径，
                    // 这里不误触发外部进程启动。
                },
            }
        }
        Ok(backends)
    }

    pub fn materialize_builtin_backends(
        &self,
        plans: &[PluginBackendPlan],
    ) -> Vec<BuiltinPluginRuntimeHandle> {
        plans
            .iter()
            .filter(|plan| plan.backend_kind == PluginBackendKind::InProcess)
            .map(|plan| BuiltinPluginRuntimeHandle::new(plan.plugin_id.clone()))
            .collect()
    }

    pub fn external_backend_health_reports(
        &self,
        backends: &mut [ExternalPluginRuntimeHandle],
    ) -> Result<Vec<PluginBackendHealthReport>> {
        backends
            .iter_mut()
            .map(ExternalPluginRuntimeHandle::health_report)
            .collect()
    }

    pub async fn reload_from_descriptors(
        &self,
        descriptors: Vec<PluginDescriptor>,
    ) -> Result<PluginHostReload> {
        self.reload_from_descriptors_with_capabilities(descriptors, &[])
            .await
    }

    pub async fn reload_from_descriptors_with_capabilities(
        &self,
        mut descriptors: Vec<PluginDescriptor>,
        capabilities: &[CapabilityWireDescriptor],
    ) -> Result<PluginHostReload> {
        Self::sort_descriptors_for_reload(&mut descriptors);
        let plans = self.backend_plans(&descriptors)?;
        let resources = resources_discover(&descriptors)?.catalog;
        self.registry.stage_candidate(descriptors.clone())?;
        let builtin_backends = self.materialize_builtin_backends(&plans);
        let external_backends = match self
            .start_external_process_backends_with_capabilities(&plans, capabilities)
            .await
        {
            Ok(backends) => backends,
            Err(error) => {
                self.registry.rollback_candidate();
                return Err(error);
            },
        };
        let snapshot = self.registry.commit_candidate().ok_or_else(|| {
            astrcode_core::AstrError::Internal("candidate commit unexpectedly failed".to_string())
        })?;
        let negotiated_plugins =
            NegotiatedPluginCatalog::from_external_backends(&external_backends);
        let mut reload = PluginHostReload {
            descriptors,
            snapshot,
            builtin_backends,
            external_backends,
            resources,
            backend_health: ExternalBackendHealthCatalog::default(),
            negotiated_plugins,
            runtime_catalog: ActivePluginRuntimeCatalog {
                snapshot_id: String::new(),
                revision: 0,
                plugin_ids: Vec::new(),
                entries: Vec::new(),
                tool_names: Vec::new(),
                hook_ids: Vec::new(),
                provider_ids: Vec::new(),
                resource_ids: Vec::new(),
                command_ids: Vec::new(),
                theme_ids: Vec::new(),
                prompt_ids: Vec::new(),
                skill_ids: Vec::new(),
                negotiated_plugins: NegotiatedPluginCatalog::default(),
            },
        };
        reload.refresh_external_backend_health(self)?;
        reload.refresh_runtime_catalog();
        Ok(reload)
    }

    pub async fn reload_with_builtin_and_loader(
        &self,
        builtin_descriptors: Vec<PluginDescriptor>,
        loader: &PluginLoader,
    ) -> Result<PluginHostReload> {
        self.reload_with_builtin_loader_and_capabilities(builtin_descriptors, loader, &[])
            .await
    }

    pub async fn reload_with_builtin_loader_and_capabilities(
        &self,
        builtin_descriptors: Vec<PluginDescriptor>,
        loader: &PluginLoader,
        capabilities: &[CapabilityWireDescriptor],
    ) -> Result<PluginHostReload> {
        let discovered_descriptors = loader.discover_descriptors()?;
        let mut descriptors = builtin_descriptors;
        descriptors.extend(discovered_descriptors);
        let reload = self
            .reload_from_descriptors_with_capabilities(descriptors, capabilities)
            .await?;
        Ok(reload)
    }

    pub async fn reload_with_external_backends(
        &self,
        loader: &PluginLoader,
    ) -> Result<PluginHostReload> {
        let descriptors = loader.discover_descriptors()?;
        self.reload_from_descriptors(descriptors).await
    }

    /// 从 loader 发现 descriptors，并将其提交为新的 active snapshot。
    pub fn reload_from_loader(&self, loader: &PluginLoader) -> Result<PluginActiveSnapshot> {
        let descriptors = loader.discover_descriptors()?;
        let _plans = self.backend_plans(&descriptors)?;
        self.registry.stage_candidate(descriptors)?;
        self.registry.commit_candidate().ok_or_else(|| {
            astrcode_core::AstrError::Internal("candidate commit unexpectedly failed".to_string())
        })
    }
}

impl Default for PluginHost {
    fn default() -> Self {
        Self::new()
    }
}

fn to_plugin_invocation_context(
    ctx: &CapabilityContext,
    capability_name: &str,
) -> InvocationContext {
    let working_dir = ctx.working_dir.to_string_lossy().into_owned();
    let request_id = ctx.request_id.clone().unwrap_or_else(|| {
        format!(
            "{}:{}:{}",
            ctx.session_id,
            capability_name,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        )
    });

    InvocationContext {
        request_id,
        trace_id: ctx.trace_id.clone(),
        session_id: Some(ctx.session_id.to_string()),
        caller: None,
        workspace: Some(WorkspaceRef {
            working_dir: Some(working_dir.clone()),
            repo_root: Some(working_dir),
            branch: None,
            metadata: Value::Null,
        }),
        deadline_ms: None,
        budget: None,
        profile: ctx.profile.clone(),
        profile_context: ctx.profile_context.clone(),
        metadata: ctx.metadata.clone(),
    }
}


#[cfg(test)]
#[path = "host_tests.rs"]
mod host_tests;
