//! # Builtin Hook Executor Registry
//!
//! 提供 `BuiltinHookExecutor` trait 和 `BuiltinHookRegistry`，作为内部擦除层。
//! Builtin plugin 作者不直接实现此 trait，而是通过 `on_input`、`on_tool_call`
//! 等函数式注册 helper 注册 handler。

use std::sync::Arc;

use astrcode_core::{HookEventKey, Result};
use astrcode_runtime_contract::hooks::{HookEffect, HookEventPayload};
use async_trait::async_trait;

use crate::hooks::{HookDispatchMode, HookFailurePolicy};

// ============================================================================
// HookExecutorRef — binding 中的执行器引用
// ============================================================================

/// Hook 执行器引用：builtin（进程内）或 external（远端 backend）。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum HookExecutorRef {
    /// 内置 handler，entry_ref 形如 `builtin://hooks/<id>`。
    Builtin(String),
    /// 外部 backend handler id。
    External(String),
}

// ============================================================================
// HookBinding — active snapshot 中的可执行 hook 条目
// ============================================================================

/// Active snapshot 中已绑定执行器的 hook 条目。
///
/// 不包含预计算 effect；effect 只在 dispatch 时由 handler 返回。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookBinding {
    pub plugin_id: String,
    pub hook_id: String,
    pub event: HookEventKey,
    pub dispatch_mode: HookDispatchMode,
    pub failure_policy: HookFailurePolicy,
    pub priority: i32,
    pub executor: HookExecutorRef,
    pub snapshot_id: String,
}

// ============================================================================
// BuiltinHookExecutor trait — 内部类型擦除层
// ============================================================================

/// 内部 hook executor trait。
///
/// 这不是 builtin plugin 作者直接实现的 API；
/// plugin 作者使用函数式 helper（如 `on_tool_call`），
/// 内部由 registry 擦除为 `Arc<dyn BuiltinHookExecutor>`。
#[async_trait]
pub trait BuiltinHookExecutor: Send + Sync {
    /// 执行 hook 并返回 effects。
    ///
    /// `context` 提供受限的只读视图和 action request 通道；
    /// `payload` 提供事件特定的 typed 输入。
    async fn execute(
        &self,
        context: HookContext,
        payload: HookEventPayload,
    ) -> Result<Vec<HookEffect>>;
}

// ============================================================================
// BuiltinHookRegistry — hook handler 注册中心
// ============================================================================

/// Builtin hook handler 注册中心。
///
/// 内部以 `entry_ref -> executor` 映射维护所有内置 handler。
/// 注册可通过 `register` 方法（接受已装箱 executor），
/// 也可通过函数式 helper（在 Task 2.5 中添加）。
pub struct BuiltinHookRegistry {
    /// entry_ref -> executor 映射。
    executors: std::collections::HashMap<String, Arc<dyn BuiltinHookExecutor>>,
    /// entry_ref -> event 映射（用于校验）。
    events: std::collections::HashMap<String, HookEventKey>,
}

impl std::fmt::Debug for BuiltinHookRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuiltinHookRegistry")
            .field("entry_refs", &self.executors.keys().collect::<Vec<_>>())
            .field("count", &self.executors.len())
            .finish()
    }
}

impl Default for BuiltinHookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl BuiltinHookRegistry {
    /// 创建一个空 registry。
    pub fn new() -> Self {
        Self {
            executors: std::collections::HashMap::new(),
            events: std::collections::HashMap::new(),
        }
    }

    /// 注册 handler executor。
    ///
    /// `entry_ref` 是 descriptor 中引用的 key，
    /// 格式通常为 `builtin://hooks/<id>`。
    pub fn register(
        &mut self,
        entry_ref: impl Into<String>,
        event: HookEventKey,
        executor: Arc<dyn BuiltinHookExecutor>,
    ) {
        let key = entry_ref.into();
        self.events.insert(key.clone(), event);
        self.executors.insert(key, executor);
    }

    /// 根据 entry_ref 查找 executor。
    pub fn get(&self, entry_ref: &str) -> Option<Arc<dyn BuiltinHookExecutor>> {
        self.executors.get(entry_ref).cloned()
    }

    /// 返回 entry_ref 对应的 event（用于校验）。
    pub fn event_for(&self, entry_ref: &str) -> Option<HookEventKey> {
        self.events.get(entry_ref).copied()
    }

    /// 返回注册的 handler 数量。
    pub fn len(&self) -> usize {
        self.executors.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.executors.is_empty()
    }

    /// 返回所有 entry_ref 的迭代器。
    pub fn entry_refs(&self) -> impl Iterator<Item = &String> {
        self.executors.keys()
    }

    // ------------------------------------------------------------------
    // 函数式注册 helper
    // ------------------------------------------------------------------

    /// 注册 `input` 事件 handler。
    pub fn on_input<F, Fut>(&mut self, id: &str, handler: F)
    where
        F: Fn(HookContext, HookEventPayload) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Vec<HookEffect>>> + Send,
    {
        self.register(
            format!("builtin://hooks/{id}"),
            HookEventKey::Input,
            Arc::new(FnHookExecutor(handler)),
        );
    }

    /// 注册 `tool_call` 事件 handler。
    pub fn on_tool_call<F, Fut>(&mut self, id: &str, handler: F)
    where
        F: Fn(HookContext, HookEventPayload) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Vec<HookEffect>>> + Send,
    {
        self.register(
            format!("builtin://hooks/{id}"),
            HookEventKey::ToolCall,
            Arc::new(FnHookExecutor(handler)),
        );
    }

    /// 注册 `tool_result` 事件 handler。
    pub fn on_tool_result<F, Fut>(&mut self, id: &str, handler: F)
    where
        F: Fn(HookContext, HookEventPayload) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Vec<HookEffect>>> + Send,
    {
        self.register(
            format!("builtin://hooks/{id}"),
            HookEventKey::ToolResult,
            Arc::new(FnHookExecutor(handler)),
        );
    }

    /// 注册 `before_provider_request` 事件 handler。
    pub fn on_before_provider_request<F, Fut>(&mut self, id: &str, handler: F)
    where
        F: Fn(HookContext, HookEventPayload) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Vec<HookEffect>>> + Send,
    {
        self.register(
            format!("builtin://hooks/{id}"),
            HookEventKey::BeforeProviderRequest,
            Arc::new(FnHookExecutor(handler)),
        );
    }

    /// 注册 `session_before_compact` 事件 handler。
    pub fn on_session_before_compact<F, Fut>(&mut self, id: &str, handler: F)
    where
        F: Fn(HookContext, HookEventPayload) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Vec<HookEffect>>> + Send,
    {
        self.register(
            format!("builtin://hooks/{id}"),
            HookEventKey::SessionBeforeCompact,
            Arc::new(FnHookExecutor(handler)),
        );
    }

    /// 注册 `model_select` 事件 handler。
    pub fn on_model_select<F, Fut>(&mut self, id: &str, handler: F)
    where
        F: Fn(HookContext, HookEventPayload) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Vec<HookEffect>>> + Send,
    {
        self.register(
            format!("builtin://hooks/{id}"),
            HookEventKey::ModelSelect,
            Arc::new(FnHookExecutor(handler)),
        );
    }
}

// ============================================================================
// FnHookExecutor — 将异步闭包包装为 BuiltinHookExecutor
// ============================================================================

/// 将 async closure 包装为 `BuiltinHookExecutor` 的内部结构。
///
/// builtin plugin 作者不直接使用此类型；通过 `registry.on_tool_call(...)` 等
/// 函数式 helper 注册 handler，内部自动擦除为 `FnHookExecutor`。
struct FnHookExecutor<F>(F);

#[async_trait]
impl<F, Fut> BuiltinHookExecutor for FnHookExecutor<F>
where
    F: Fn(HookContext, HookEventPayload) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<Vec<HookEffect>>> + Send,
{
    async fn execute(
        &self,
        context: HookContext,
        payload: HookEventPayload,
    ) -> Result<Vec<HookEffect>> {
        (self.0)(context, payload).await
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::Result;
    use astrcode_runtime_contract::hooks::{HookEffect, HookEventPayload};

    use super::{BuiltinHookExecutor, BuiltinHookRegistry, HookContext, HookExecutorRef};

    #[tokio::test]
    async fn registry_register_and_lookup() {
        let mut registry = BuiltinHookRegistry::new();
        let executor = std::sync::Arc::new(TestExecutor);

        registry.register(
            "builtin://hooks/test-1",
            astrcode_core::HookEventKey::ToolCall,
            executor.clone(),
        );

        assert_eq!(registry.len(), 1);
        assert!(registry.get("builtin://hooks/test-1").is_some());
        assert_eq!(
            registry.event_for("builtin://hooks/test-1"),
            Some(astrcode_core::HookEventKey::ToolCall),
        );
    }

    #[tokio::test]
    async fn registry_functional_helper_on_tool_call() {
        let mut registry = BuiltinHookRegistry::new();

        registry.on_tool_call("block-writes", |_ctx, _payload| async move {
            Ok(vec![HookEffect::BlockToolResult {
                tool_call_id: "test".to_string(),
                reason: "blocked".to_string(),
            }])
        });

        let executor = registry
            .get("builtin://hooks/block-writes")
            .expect("executor should be registered");

        let effects = executor
            .execute(
                HookContext::new(),
                HookEventPayload::from_value(
                    &astrcode_core::HookEventKey::ToolCall,
                    &serde_json::json!({}),
                ),
            )
            .await
            .expect("executor should succeed");

        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], HookEffect::BlockToolResult { .. }));
    }

    #[tokio::test]
    async fn registry_functional_helper_on_input() {
        let mut registry = BuiltinHookRegistry::new();

        registry.on_input("switch-mode", |_ctx, _payload| async move {
            Ok(vec![HookEffect::SwitchMode {
                mode_id: "plan".to_string(),
            }])
        });

        assert_eq!(registry.len(), 1);
        let executor = registry
            .get("builtin://hooks/switch-mode")
            .expect("should exist");
        let effects = executor
            .execute(
                HookContext::new(),
                HookEventPayload::from_value(
                    &astrcode_core::HookEventKey::Input,
                    &serde_json::json!({}),
                ),
            )
            .await
            .expect("should execute");
        assert!(matches!(effects[0], HookEffect::SwitchMode { .. }));
    }

    #[tokio::test]
    async fn registry_empty_returns_none() {
        let registry = BuiltinHookRegistry::new();
        assert!(registry.get("builtin://hooks/nonexistent").is_none());
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn hook_context_builder_defaults() {
        let ctx = HookContext::new();
        assert!(ctx.snapshot_id.is_none());
        assert!(ctx.session_id.is_none());
        assert!(ctx.host_view.is_none());
    }

    #[test]
    fn hook_context_builder_with_fields() {
        let ctx = HookContext::new()
            .with_snapshot_id("snap-1")
            .with_session_id("session-1")
            .with_turn_id("turn-1")
            .with_agent_id("agent-1")
            .with_current_mode("plan");

        assert_eq!(ctx.snapshot_id.as_deref(), Some("snap-1"));
        assert_eq!(ctx.session_id.as_deref(), Some("session-1"));
        assert_eq!(ctx.current_mode.as_deref(), Some("plan"));
    }

    #[test]
    fn hook_executor_ref_equality() {
        let builtin_a = HookExecutorRef::Builtin("builtin://hooks/a".to_string());
        let builtin_b = HookExecutorRef::Builtin("builtin://hooks/a".to_string());
        let external = HookExecutorRef::External("ext-1".to_string());

        assert_eq!(builtin_a, builtin_b);
        assert_ne!(builtin_a, external);
    }

    #[test]
    fn allowed_effects_for_event_coverage() {
        let events = [
            (
                "input",
                &[
                    "Continue",
                    "Diagnostic",
                    "TransformInput",
                    "HandledInput",
                    "SwitchMode",
                ][..],
            ),
            (
                "tool_call",
                &[
                    "Continue",
                    "Diagnostic",
                    "MutateToolArgs",
                    "BlockToolResult",
                    "RequireApproval",
                    "CancelTurn",
                ][..],
            ),
            (
                "tool_result",
                &["Continue", "Diagnostic", "OverrideToolResult"][..],
            ),
            (
                "model_select",
                &["Continue", "Diagnostic", "ModelHint", "DenyModelSelect"][..],
            ),
            ("unknown", &["Continue", "Diagnostic"]),
        ];

        for (event, expected) in &events {
            let allowed = astrcode_runtime_contract::hooks::allowed_effects_for_event(event);
            assert_eq!(allowed, *expected, "mismatch for event '{event}'");
        }
    }

    /// A simple executor that returns Continue for any input.
    struct TestExecutor;

    #[async_trait::async_trait]
    impl BuiltinHookExecutor for TestExecutor {
        async fn execute(
            &self,
            _context: HookContext,
            _payload: HookEventPayload,
        ) -> Result<Vec<HookEffect>> {
            Ok(vec![HookEffect::Continue])
        }
    }
}

// ============================================================================
// HookContext — 受限的 hook 执行上下文
// ============================================================================

/// 受限的 hook 执行上下文。
///
/// 提供 typed metadata、只读宿主视图、取消状态和受限 action request。
/// 不暴露 `EventStore`、mutable session state 或 snapshot mutation。
#[derive(Debug, Clone, Default)]
pub struct HookContext {
    pub snapshot_id: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub agent_id: Option<String>,
    pub current_mode: Option<String>,
    pub cancel_state: Option<bool>,
    pub host_view: Option<HookHostView>,
}

/// 受限的宿主只读视图。
#[derive(Debug, Clone, Default)]
pub struct HookHostView {
    pub session_meta: Option<serde_json::Value>,
}

impl HookContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_snapshot_id(mut self, snapshot_id: impl Into<String>) -> Self {
        self.snapshot_id = Some(snapshot_id.into());
        self
    }

    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    pub fn with_turn_id(mut self, turn_id: impl Into<String>) -> Self {
        self.turn_id = Some(turn_id.into());
        self
    }

    pub fn with_agent_id(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }

    pub fn with_current_mode(mut self, mode: impl Into<String>) -> Self {
        self.current_mode = Some(mode.into());
        self
    }
}
