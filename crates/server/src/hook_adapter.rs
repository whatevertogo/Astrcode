//! # PluginHost Hook Adapter
//!
//! 将 plugin-host 的 dispatch core 包装为 `agent-runtime` 和 `host-session`
//! 可消费的 hook dispatcher，避免横向依赖。

use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
};

use astrcode_agent_runtime::hook_dispatch::{HookDispatchRequest, HookDispatcher};
use astrcode_core::{HookEventKey, Result};
use astrcode_host_session::HookDispatch as HostSessionHookDispatch;
use astrcode_plugin_host::{BuiltinHookRegistry, HookBinding, HookContext, dispatch_hooks};
use astrcode_runtime_contract::hooks::{HookDispatchOutcome, HookEffect, HookEventPayload};
use async_trait::async_trait;

/// 将 plugin-host 的 dispatch core 包装为 agent-runtime 的 HookDispatcher。
pub struct PluginHostHookDispatcher {
    snapshots: Arc<RwLock<HookSnapshotStore>>,
    /// builtin hook executor registry。
    registry: Arc<BuiltinHookRegistry>,
}

#[derive(Debug)]
struct HookSnapshotStore {
    active_snapshot_id: String,
    bindings_by_snapshot_id: BTreeMap<String, Arc<Vec<HookBinding>>>,
}

impl PluginHostHookDispatcher {
    pub fn new(
        snapshot_id: impl Into<String>,
        bindings: Arc<Vec<HookBinding>>,
        registry: Arc<BuiltinHookRegistry>,
    ) -> Self {
        let snapshot_id = snapshot_id.into();
        let mut bindings_by_snapshot_id = BTreeMap::new();
        bindings_by_snapshot_id.insert(snapshot_id.clone(), bindings);
        Self {
            snapshots: Arc::new(RwLock::new(HookSnapshotStore {
                active_snapshot_id: snapshot_id,
                bindings_by_snapshot_id,
            })),
            registry,
        }
    }

    pub fn active_snapshot_id(&self) -> String {
        self.snapshots
            .read()
            .expect("plugin hook snapshot lock poisoned")
            .active_snapshot_id
            .clone()
    }

    pub fn replace_active_snapshot(
        &self,
        snapshot_id: impl Into<String>,
        bindings: Vec<HookBinding>,
    ) {
        let snapshot_id = snapshot_id.into();
        let mut snapshots = self
            .snapshots
            .write()
            .expect("plugin hook snapshot lock poisoned");
        snapshots
            .bindings_by_snapshot_id
            .insert(snapshot_id.clone(), Arc::new(bindings));
        snapshots.active_snapshot_id = snapshot_id;
    }

    fn bindings_for_snapshot(&self, snapshot_id: &str) -> Result<Arc<Vec<HookBinding>>> {
        let snapshots = self
            .snapshots
            .read()
            .expect("plugin hook snapshot lock poisoned");
        snapshots
            .bindings_by_snapshot_id
            .get(snapshot_id)
            .cloned()
            .ok_or_else(|| {
                astrcode_core::AstrError::Validation(format!(
                    "hook snapshot '{}' is not available in server hook dispatcher",
                    snapshot_id
                ))
            })
    }

    fn active_bindings(&self) -> (String, Arc<Vec<HookBinding>>) {
        let snapshots = self
            .snapshots
            .read()
            .expect("plugin hook snapshot lock poisoned");
        let snapshot_id = snapshots.active_snapshot_id.clone();
        let bindings = snapshots
            .bindings_by_snapshot_id
            .get(&snapshot_id)
            .cloned()
            .unwrap_or_else(|| Arc::new(Vec::new()));
        (snapshot_id, bindings)
    }
}

#[async_trait]
impl HookDispatcher for PluginHostHookDispatcher {
    async fn dispatch_hook(&self, request: HookDispatchRequest) -> Result<HookDispatchOutcome> {
        let bindings = self.bindings_for_snapshot(&request.snapshot_id)?;
        let mut context = HookContext::new()
            .with_snapshot_id(&request.snapshot_id)
            .with_session_id(&request.session_id)
            .with_turn_id(&request.turn_id)
            .with_agent_id(&request.agent_id);
        if let Some(current_mode) = request.payload.current_mode() {
            context = context.with_current_mode(current_mode);
        }

        let effects = dispatch_hooks(
            request.event,
            request.payload,
            context,
            &bindings,
            &self.registry,
        )
        .await?;

        Ok(HookDispatchOutcome { effects })
    }
}

#[async_trait]
impl HostSessionHookDispatch for PluginHostHookDispatcher {
    async fn dispatch_hook(
        &self,
        event: HookEventKey,
        payload: HookEventPayload,
    ) -> Result<Vec<HookEffect>> {
        let (snapshot_id, bindings) = self.active_bindings();
        let mut context = HookContext::new().with_snapshot_id(&snapshot_id);
        context = enrich_context_from_payload(context, &payload);
        dispatch_hooks(event, payload, context, &bindings, &self.registry).await
    }
}

fn enrich_context_from_payload(
    mut context: HookContext,
    payload: &HookEventPayload,
) -> HookContext {
    match payload {
        HookEventPayload::Input { session_id, .. }
        | HookEventPayload::SessionBeforeCompact { session_id, .. }
        | HookEventPayload::ModelSelect { session_id, .. } => {
            context = context.with_session_id(session_id);
        },
        HookEventPayload::Context {
            session_id,
            turn_id,
            agent_id,
            ..
        }
        | HookEventPayload::BeforeAgentStart {
            session_id,
            turn_id,
            agent_id,
            ..
        }
        | HookEventPayload::TurnStart {
            session_id,
            turn_id,
            agent_id,
            ..
        }
        | HookEventPayload::TurnEnd {
            session_id,
            turn_id,
            agent_id,
            ..
        } => {
            context = context
                .with_session_id(session_id)
                .with_turn_id(turn_id)
                .with_agent_id(agent_id);
        },
        HookEventPayload::BeforeProviderRequest {
            session_id,
            turn_id,
            ..
        }
        | HookEventPayload::ToolResult {
            session_id,
            turn_id,
            ..
        } => {
            context = context.with_session_id(session_id).with_turn_id(turn_id);
        },
        HookEventPayload::ToolCall {
            session_id,
            turn_id,
            agent_id,
            ..
        } => {
            context = context
                .with_session_id(session_id)
                .with_turn_id(turn_id)
                .with_agent_id(agent_id);
        },
        HookEventPayload::ResourcesDiscover { snapshot_id, .. } => {
            context = context.with_snapshot_id(snapshot_id);
        },
    }
    if let Some(current_mode) = payload.current_mode() {
        context = context.with_current_mode(current_mode);
    }
    context
}
