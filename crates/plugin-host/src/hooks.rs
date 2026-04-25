use astrcode_core::{AstrError, HookEventKey, Result};
use astrcode_runtime_contract::hooks::{HookEffect, HookEventPayload, allowed_effects_for_event};

use crate::builtin_hooks::{BuiltinHookRegistry, HookBinding, HookContext, HookExecutorRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookStage {
    Runtime,
    Host,
    Resource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookDispatchMode {
    Sequential,
    Cancellable,
    Intercept,
    Modify,
    Pipeline,
    ShortCircuit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookFailurePolicy {
    FailClosed,
    FailOpen,
    ReportOnly,
}

pub const SUPPORTED_HOOK_EVENTS: &[HookEventKey] = &[
    HookEventKey::Input,
    HookEventKey::Context,
    HookEventKey::BeforeAgentStart,
    HookEventKey::BeforeProviderRequest,
    HookEventKey::ToolCall,
    HookEventKey::ToolResult,
    HookEventKey::TurnStart,
    HookEventKey::TurnEnd,
    HookEventKey::SessionBeforeCompact,
    HookEventKey::ResourcesDiscover,
    HookEventKey::ModelSelect,
];

/// 生产路径：使用 active snapshot 中的 hook bindings 执行 typed hook dispatch。
///
/// 每个 binding 对应一个已注册的 handler（builtin 或 external），
/// handler 被实际调用并返回 typed `HookEffect`。
pub async fn dispatch_hooks(
    event: HookEventKey,
    payload: HookEventPayload,
    context: HookContext,
    bindings: &[HookBinding],
    registry: &BuiltinHookRegistry,
) -> Result<Vec<HookEffect>> {
    if !SUPPORTED_HOOK_EVENTS.contains(&event) {
        return Err(AstrError::Validation(format!(
            "hook event '{event:?}' is not supported by plugin-host hook bus"
        )));
    }
    if payload.event_key() != event {
        return Err(AstrError::Validation(format!(
            "hook payload event '{:?}' does not match dispatch event '{event:?}'",
            payload.event_key()
        )));
    }

    let mut matching: Vec<&HookBinding> = bindings.iter().filter(|b| b.event == event).collect();
    matching.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.hook_id.cmp(&b.hook_id))
    });

    let mut collected_effects: Vec<HookEffect> = Vec::new();
    let mut should_stop_dispatch = false;

    let event_name = hook_event_key_name(event).to_string();
    let allowed = allowed_effects_for_event(&event_name);

    for binding in &matching {
        if should_stop_dispatch {
            break;
        }

        let outcome = match &binding.executor {
            HookExecutorRef::Builtin(entry_ref) => match registry.get(entry_ref) {
                Some(executor) => {
                    let result = executor.execute(context.clone(), payload.clone()).await;
                    match result {
                        Ok(contract_effects) => DispatchExecOutcome::Effects(contract_effects),
                        Err(error) => dispatch_exec_error(binding, &error),
                    }
                },
                None => dispatch_exec_missing(binding),
            },
            HookExecutorRef::External(_handler_id) => dispatch_exec_external_not_connected(binding),
        };

        match outcome {
            DispatchExecOutcome::Fail(reason) => {
                return Err(AstrError::Validation(reason));
            },
            DispatchExecOutcome::Report(message) => {
                collected_effects.push(HookEffect::Diagnostic { message });
                continue;
            },
            DispatchExecOutcome::Effects(contract_effects) => {
                for effect in contract_effects {
                    let effect_name = effect_variant_name(&effect);
                    if !allowed.contains(&effect_name) {
                        match binding.failure_policy {
                            HookFailurePolicy::FailClosed => {
                                return Err(AstrError::Validation(format!(
                                    "hook '{}' returned invalid effect '{effect_name}' for event \
                                     '{event_name}'",
                                    binding.hook_id
                                )));
                            },
                            HookFailurePolicy::FailOpen | HookFailurePolicy::ReportOnly => {
                                collected_effects.push(HookEffect::Diagnostic {
                                    message: format!(
                                        "invalid effect '{effect_name}' for event '{event_name}'"
                                    ),
                                });
                                continue;
                            },
                        }
                    }

                    let should_stop = should_stop_after_effect(binding.dispatch_mode, &effect);
                    collected_effects.push(effect);

                    if should_stop {
                        should_stop_dispatch = true;
                        break;
                    }
                }
            },
        }
    }

    Ok(collected_effects)
}

fn effect_variant_name(effect: &HookEffect) -> &'static str {
    match effect {
        HookEffect::Continue => "Continue",
        HookEffect::Diagnostic { .. } => "Diagnostic",
        HookEffect::TransformInput { .. } => "TransformInput",
        HookEffect::HandledInput { .. } => "HandledInput",
        HookEffect::SwitchMode { .. } => "SwitchMode",
        HookEffect::ModifyProviderRequest { .. } => "ModifyProviderRequest",
        HookEffect::DenyProviderRequest { .. } => "DenyProviderRequest",
        HookEffect::MutateToolArgs { .. } => "MutateToolArgs",
        HookEffect::BlockToolResult { .. } => "BlockToolResult",
        HookEffect::RequireApproval { .. } => "RequireApproval",
        HookEffect::OverrideToolResult { .. } => "OverrideToolResult",
        HookEffect::CancelTurn { .. } => "CancelTurn",
        HookEffect::CancelCompact { .. } => "CancelCompact",
        HookEffect::OverrideCompactInput { .. } => "OverrideCompactInput",
        HookEffect::ProvideCompactSummary { .. } => "ProvideCompactSummary",
        HookEffect::ResourcePath { .. } => "ResourcePath",
        HookEffect::ModelHint { .. } => "ModelHint",
        HookEffect::DenyModelSelect { .. } => "DenyModelSelect",
    }
}

enum DispatchExecOutcome {
    Report(String),
    Fail(String),
    Effects(Vec<HookEffect>),
}

fn dispatch_exec_error(
    binding: &HookBinding,
    error: &astrcode_core::AstrError,
) -> DispatchExecOutcome {
    match binding.failure_policy {
        HookFailurePolicy::FailClosed => DispatchExecOutcome::Fail(format!(
            "hook '{}' execution failed: {}",
            binding.hook_id, error
        )),
        HookFailurePolicy::FailOpen | HookFailurePolicy::ReportOnly => DispatchExecOutcome::Report(
            format!("hook '{}' execution failed: {}", binding.hook_id, error),
        ),
    }
}

fn dispatch_exec_missing(binding: &HookBinding) -> DispatchExecOutcome {
    match binding.failure_policy {
        HookFailurePolicy::FailClosed => {
            DispatchExecOutcome::Fail(format!("hook '{}' executor not found", binding.hook_id))
        },
        HookFailurePolicy::FailOpen | HookFailurePolicy::ReportOnly => {
            DispatchExecOutcome::Report(format!("hook '{}' executor not found", binding.hook_id))
        },
    }
}

fn dispatch_exec_external_not_connected(binding: &HookBinding) -> DispatchExecOutcome {
    match binding.failure_policy {
        HookFailurePolicy::FailClosed => DispatchExecOutcome::Fail(format!(
            "external hook '{}' is not connected to a plugin runtime",
            binding.hook_id
        )),
        HookFailurePolicy::FailOpen | HookFailurePolicy::ReportOnly => {
            DispatchExecOutcome::Report(format!(
                "external hook '{}' is not connected to a plugin runtime",
                binding.hook_id
            ))
        },
    }
}

fn should_stop_after_effect(dispatch_mode: HookDispatchMode, effect: &HookEffect) -> bool {
    if effect.is_terminal() {
        return true;
    }

    match dispatch_mode {
        HookDispatchMode::Cancellable | HookDispatchMode::Intercept => matches!(
            effect,
            HookEffect::BlockToolResult { .. }
                | HookEffect::RequireApproval { .. }
                | HookEffect::HandledInput { .. }
        ),
        HookDispatchMode::ShortCircuit => !effect.is_continue(),
        HookDispatchMode::Sequential | HookDispatchMode::Modify | HookDispatchMode::Pipeline => {
            false
        },
    }
}

fn hook_event_key_name(event: HookEventKey) -> &'static str {
    match event {
        HookEventKey::Input => "input",
        HookEventKey::Context => "context",
        HookEventKey::BeforeAgentStart => "before_agent_start",
        HookEventKey::BeforeProviderRequest => "before_provider_request",
        HookEventKey::ToolCall => "tool_call",
        HookEventKey::ToolResult => "tool_result",
        HookEventKey::TurnStart => "turn_start",
        HookEventKey::TurnEnd => "turn_end",
        HookEventKey::SessionBeforeCompact => "session_before_compact",
        HookEventKey::ResourcesDiscover => "resources_discover",
        HookEventKey::ModelSelect => "model_select",
    }
}

#[cfg(test)]
mod tests {
    use astrcode_runtime_contract::hooks::{HookEffect, HookEventPayload};

    use super::*;
    use crate::builtin_hooks::{BuiltinHookRegistry, HookBinding, HookContext, HookExecutorRef};

    fn binding(id: &str, mode: HookDispatchMode, priority: i32) -> HookBinding {
        HookBinding {
            plugin_id: "builtin-test".to_string(),
            hook_id: id.to_string(),
            event: HookEventKey::ToolCall,
            dispatch_mode: mode,
            failure_policy: HookFailurePolicy::FailClosed,
            priority,
            executor: HookExecutorRef::Builtin(format!("builtin://hooks/{id}")),
            snapshot_id: "snapshot-1".to_string(),
        }
    }

    fn tool_call_payload(id: &str) -> HookEventPayload {
        HookEventPayload::from_value(
            &HookEventKey::ToolCall,
            &serde_json::json!({
                "sessionId": "session-1",
                "turnId": "turn-1",
                "agentId": "agent-1",
                "toolCallId": id,
                "toolName": "readFile",
                "args": { "path": "README.md" },
                "stepIndex": 0
            }),
        )
    }

    #[tokio::test]
    async fn dispatch_hooks_preserves_typed_payload() {
        let mut registry = BuiltinHookRegistry::new();
        registry.on_tool_call("capture", |_ctx, payload| async move {
            let message = match payload {
                HookEventPayload::ToolCall { tool_call_id, .. } => tool_call_id,
                _ => "wrong-payload".to_string(),
            };
            Ok(vec![HookEffect::Diagnostic { message }])
        });

        let effects = dispatch_hooks(
            HookEventKey::ToolCall,
            tool_call_payload("call-1"),
            HookContext::new(),
            &[binding("capture", HookDispatchMode::Sequential, 0)],
            &registry,
        )
        .await
        .expect("hook dispatch should succeed");

        assert!(matches!(
            effects.as_slice(),
            [HookEffect::Diagnostic { message }] if message == "call-1"
        ));
    }

    #[tokio::test]
    async fn fail_closed_invalid_effect_returns_error() {
        let mut registry = BuiltinHookRegistry::new();
        registry.on_tool_call("bad-effect", |_ctx, _payload| async move {
            Ok(vec![HookEffect::ModelHint {
                model: "other".to_string(),
            }])
        });

        let error = dispatch_hooks(
            HookEventKey::ToolCall,
            tool_call_payload("call-1"),
            HookContext::new(),
            &[binding("bad-effect", HookDispatchMode::Sequential, 0)],
            &registry,
        )
        .await
        .expect_err("invalid effect should fail closed");

        assert!(error.to_string().contains("invalid effect 'ModelHint'"));
    }

    #[tokio::test]
    async fn cancellable_dispatch_stops_after_blocking_effect() {
        let mut registry = BuiltinHookRegistry::new();
        registry.on_tool_call("blocker", |_ctx, payload| async move {
            let tool_call_id = match payload {
                HookEventPayload::ToolCall { tool_call_id, .. } => tool_call_id,
                _ => String::new(),
            };
            Ok(vec![HookEffect::BlockToolResult {
                tool_call_id,
                reason: "blocked".to_string(),
            }])
        });
        registry.on_tool_call("later", |_ctx, _payload| async move {
            Ok(vec![HookEffect::Diagnostic {
                message: "should-not-run".to_string(),
            }])
        });

        let effects = dispatch_hooks(
            HookEventKey::ToolCall,
            tool_call_payload("call-1"),
            HookContext::new(),
            &[
                binding("blocker", HookDispatchMode::Cancellable, 10),
                binding("later", HookDispatchMode::Sequential, 0),
            ],
            &registry,
        )
        .await
        .expect("hook dispatch should succeed");

        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], HookEffect::BlockToolResult { .. }));
    }
}
