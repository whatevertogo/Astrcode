//! # PluginHost Hook Adapter
//!
//! 将 plugin-host 的 dispatch core 包装为 `agent-runtime` 和 `host-session`
//! 可消费的 hook dispatcher，避免横向依赖。

use std::sync::Arc;

use astrcode_agent_runtime::hook_dispatch::{HookDispatchRequest, HookDispatcher};
use astrcode_core::Result;
use astrcode_plugin_host::{BuiltinHookRegistry, HookBinding, HookContext, dispatch_hooks};
use astrcode_runtime_contract::hooks::HookDispatchOutcome;
use async_trait::async_trait;

/// 将 plugin-host 的 dispatch core 包装为 agent-runtime 的 HookDispatcher。
pub struct PluginHostHookDispatcher {
    /// 当前 active snapshot 中的 hook bindings。
    bindings: Arc<Vec<HookBinding>>,
    /// builtin hook executor registry。
    registry: Arc<BuiltinHookRegistry>,
}

impl PluginHostHookDispatcher {
    pub fn new(bindings: Arc<Vec<HookBinding>>, registry: Arc<BuiltinHookRegistry>) -> Self {
        Self { bindings, registry }
    }
}

#[async_trait]
impl HookDispatcher for PluginHostHookDispatcher {
    async fn dispatch_hook(&self, request: HookDispatchRequest) -> Result<HookDispatchOutcome> {
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
            &self.bindings,
            &self.registry,
        )
        .await?;

        Ok(HookDispatchOutcome { effects })
    }
}
