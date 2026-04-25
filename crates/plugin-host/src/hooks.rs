use std::sync::atomic::{AtomicU64, Ordering};

use astrcode_core::{AstrError, HookEventKey, Result};
use astrcode_protocol::plugin::{HookDiagnosticWire, HookDispatchMessage, HookEffectWire};
use astrcode_runtime_contract::hooks::{HookEffect, HookEventPayload, allowed_effects_for_event};
use async_trait::async_trait;
use serde_json::Value;

use crate::builtin_hooks::{BuiltinHookRegistry, HookBinding, HookContext, HookExecutorRef};

static EXTERNAL_HOOK_CORRELATION_COUNTER: AtomicU64 = AtomicU64::new(1);

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

#[derive(Debug, Clone)]
pub struct ExternalHookDispatchRequest {
    pub binding: HookBinding,
    pub context: HookContext,
    pub payload: HookEventPayload,
}

#[async_trait]
pub trait ExternalHookDispatcher: Send {
    async fn dispatch_hook(
        &mut self,
        request: ExternalHookDispatchRequest,
    ) -> Result<Vec<HookEffect>>;
}

impl ExternalHookDispatchRequest {
    pub fn into_protocol_message(self) -> Result<HookDispatchMessage> {
        let payload = hook_payload_to_wire(&self.payload)?;
        let sequence = EXTERNAL_HOOK_CORRELATION_COUNTER.fetch_add(1, Ordering::Relaxed);
        Ok(HookDispatchMessage {
            correlation_id: format!(
                "hook:{}:{}:{}:{}",
                self.binding.snapshot_id, self.binding.plugin_id, self.binding.hook_id, sequence
            ),
            snapshot_id: self.binding.snapshot_id,
            plugin_id: self.binding.plugin_id,
            hook_id: match self.binding.executor {
                HookExecutorRef::External(handler_id) => handler_id,
                HookExecutorRef::Builtin(_) => self.binding.hook_id,
            },
            event: hook_event_key_name(self.payload.event_key()).to_string(),
            payload,
        })
    }
}

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
    dispatch_hooks_with_external(event, payload, context, bindings, registry, None).await
}

pub async fn dispatch_hooks_with_external(
    event: HookEventKey,
    payload: HookEventPayload,
    context: HookContext,
    bindings: &[HookBinding],
    registry: &BuiltinHookRegistry,
    mut external_dispatcher: Option<&mut dyn ExternalHookDispatcher>,
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
            HookExecutorRef::External(_) => match external_dispatcher.as_deref_mut() {
                Some(dispatcher) => {
                    match dispatcher
                        .dispatch_hook(ExternalHookDispatchRequest {
                            binding: (*binding).clone(),
                            context: context.clone(),
                            payload: payload.clone(),
                        })
                        .await
                    {
                        Ok(effects) => DispatchExecOutcome::Effects(effects),
                        Err(error) => dispatch_exec_error(binding, &error),
                    }
                },
                None => dispatch_exec_external_not_connected(binding),
            },
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

pub fn hook_effects_from_wire(
    effects: Vec<HookEffectWire>,
    diagnostics: Vec<HookDiagnosticWire>,
) -> Result<Vec<HookEffect>> {
    let mut mapped = diagnostics
        .into_iter()
        .map(|diagnostic| HookEffect::Diagnostic {
            message: match diagnostic.severity {
                Some(severity) => format!("{severity}: {}", diagnostic.message),
                None => diagnostic.message,
            },
        })
        .collect::<Vec<_>>();
    for effect in effects {
        mapped.push(hook_effect_from_wire(effect)?);
    }
    Ok(mapped)
}

pub fn hook_effect_from_wire(effect: HookEffectWire) -> Result<HookEffect> {
    let payload = effect.payload;
    match effect.kind.as_str() {
        "Continue" => Ok(HookEffect::Continue),
        "Diagnostic" => Ok(HookEffect::Diagnostic {
            message: required_string(&payload, "message", &effect.kind)?,
        }),
        "TransformInput" => Ok(HookEffect::TransformInput {
            text: required_string(&payload, "text", &effect.kind)?,
        }),
        "HandledInput" => Ok(HookEffect::HandledInput {
            response: required_string(&payload, "response", &effect.kind)?,
        }),
        "SwitchMode" => Ok(HookEffect::SwitchMode {
            mode_id: required_string(&payload, "modeId", &effect.kind)?,
        }),
        "ModifyProviderRequest" => Ok(HookEffect::ModifyProviderRequest {
            request: required_value(&payload, "request", &effect.kind)?.clone(),
        }),
        "DenyProviderRequest" => Ok(HookEffect::DenyProviderRequest {
            reason: required_string(&payload, "reason", &effect.kind)?,
        }),
        "MutateToolArgs" => Ok(HookEffect::MutateToolArgs {
            tool_call_id: required_string(&payload, "toolCallId", &effect.kind)?,
            args: required_value(&payload, "args", &effect.kind)?.clone(),
        }),
        "BlockToolResult" => Ok(HookEffect::BlockToolResult {
            tool_call_id: required_string(&payload, "toolCallId", &effect.kind)?,
            reason: required_string(&payload, "reason", &effect.kind)?,
        }),
        "RequireApproval" => Ok(HookEffect::RequireApproval {
            request_id: required_string(&payload, "requestId", &effect.kind)?,
            reason: required_string(&payload, "reason", &effect.kind)?,
        }),
        "OverrideToolResult" => Ok(HookEffect::OverrideToolResult {
            tool_call_id: required_string(&payload, "toolCallId", &effect.kind)?,
            result: required_value(&payload, "result", &effect.kind)?.clone(),
            ok: optional_bool(&payload, "ok").unwrap_or(true),
        }),
        "CancelTurn" => Ok(HookEffect::CancelTurn {
            reason: required_string(&payload, "reason", &effect.kind)?,
        }),
        "CancelCompact" => Ok(HookEffect::CancelCompact {
            reason: required_string(&payload, "reason", &effect.kind)?,
        }),
        "OverrideCompactInput" => Ok(HookEffect::OverrideCompactInput {
            reason: parse_required_value(&payload, "reason", &effect.kind)?,
            messages: parse_required_value(&payload, "messages", &effect.kind)?,
        }),
        "ProvideCompactSummary" => Ok(HookEffect::ProvideCompactSummary {
            summary: required_string(&payload, "summary", &effect.kind)?,
        }),
        "ResourcePath" => Ok(HookEffect::ResourcePath {
            path: required_string(&payload, "path", &effect.kind)?,
        }),
        "ModelHint" => Ok(HookEffect::ModelHint {
            model: required_string(&payload, "model", &effect.kind)?,
        }),
        "DenyModelSelect" => Ok(HookEffect::DenyModelSelect {
            reason: required_string(&payload, "reason", &effect.kind)?,
        }),
        other => Err(AstrError::Validation(format!(
            "unknown hook effect kind '{other}'"
        ))),
    }
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

fn hook_payload_to_wire(payload: &HookEventPayload) -> Result<Value> {
    Ok(match payload {
        HookEventPayload::Input {
            session_id,
            source,
            text,
            images,
            current_mode,
        } => serde_json::json!({
            "sessionId": session_id,
            "source": source,
            "text": text,
            "images": images,
            "currentMode": current_mode,
        }),
        HookEventPayload::Context {
            session_id,
            turn_id,
            agent_id,
            step_index,
            message_count,
            current_mode,
        } => serde_json::json!({
            "sessionId": session_id,
            "turnId": turn_id,
            "agentId": agent_id,
            "stepIndex": step_index,
            "messageCount": message_count,
            "currentMode": current_mode,
        }),
        HookEventPayload::BeforeAgentStart {
            session_id,
            turn_id,
            agent_id,
            step_index,
            message_count,
            current_mode,
        } => serde_json::json!({
            "sessionId": session_id,
            "turnId": turn_id,
            "agentId": agent_id,
            "stepIndex": step_index,
            "messageCount": message_count,
            "currentMode": current_mode,
        }),
        HookEventPayload::BeforeProviderRequest {
            session_id,
            turn_id,
            provider_ref,
            model_ref,
            request,
            current_mode,
        } => serde_json::json!({
            "sessionId": session_id,
            "turnId": turn_id,
            "providerRef": provider_ref,
            "modelRef": model_ref,
            "request": request,
            "currentMode": current_mode,
        }),
        HookEventPayload::ToolCall {
            session_id,
            turn_id,
            agent_id,
            tool_call_id,
            tool_name,
            args,
            capability_spec,
            working_dir,
            current_mode,
            step_index,
        } => serde_json::json!({
            "sessionId": session_id,
            "turnId": turn_id,
            "agentId": agent_id,
            "toolCallId": tool_call_id,
            "toolName": tool_name,
            "args": args,
            "capabilitySpec": capability_spec,
            "workingDir": working_dir,
            "currentMode": current_mode,
            "stepIndex": step_index,
        }),
        HookEventPayload::ToolResult {
            session_id,
            turn_id,
            tool_call_id,
            tool_name,
            args,
            result,
            ok,
            current_mode,
        } => serde_json::json!({
            "sessionId": session_id,
            "turnId": turn_id,
            "toolCallId": tool_call_id,
            "toolName": tool_name,
            "args": args,
            "result": result,
            "ok": ok,
            "currentMode": current_mode,
        }),
        HookEventPayload::SessionBeforeCompact {
            session_id,
            reason,
            messages,
            settings,
            current_mode,
        } => serde_json::json!({
            "sessionId": session_id,
            "reason": reason,
            "messages": messages,
            "settings": settings,
            "currentMode": current_mode,
        }),
        HookEventPayload::ResourcesDiscover {
            snapshot_id,
            cwd,
            reason,
        } => serde_json::json!({
            "snapshotId": snapshot_id,
            "cwd": cwd,
            "reason": reason,
        }),
        HookEventPayload::ModelSelect {
            session_id,
            current_model,
            candidate_model,
            reason,
        } => serde_json::json!({
            "sessionId": session_id,
            "currentModel": current_model,
            "candidateModel": candidate_model,
            "reason": reason,
        }),
        HookEventPayload::TurnStart {
            session_id,
            turn_id,
            agent_id,
            current_mode,
        } => serde_json::json!({
            "sessionId": session_id,
            "turnId": turn_id,
            "agentId": agent_id,
            "currentMode": current_mode,
        }),
        HookEventPayload::TurnEnd {
            session_id,
            turn_id,
            agent_id,
            current_mode,
        } => serde_json::json!({
            "sessionId": session_id,
            "turnId": turn_id,
            "agentId": agent_id,
            "currentMode": current_mode,
        }),
    })
}

fn required_value<'a>(payload: &'a Value, field: &str, kind: &str) -> Result<&'a Value> {
    payload.get(field).ok_or_else(|| {
        AstrError::Validation(format!(
            "hook effect '{kind}' missing required payload field '{field}'"
        ))
    })
}

fn required_string(payload: &Value, field: &str, kind: &str) -> Result<String> {
    required_value(payload, field, kind)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| {
            AstrError::Validation(format!(
                "hook effect '{kind}' payload field '{field}' must be a string"
            ))
        })
}

fn optional_bool(payload: &Value, field: &str) -> Option<bool> {
    payload.get(field).and_then(Value::as_bool)
}

fn parse_required_value<T>(payload: &Value, field: &str, kind: &str) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(required_value(payload, field, kind)?.clone()).map_err(|error| {
        AstrError::Validation(format!(
            "hook effect '{kind}' payload field '{field}' has invalid shape: {error}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use astrcode_protocol::plugin::{HookDiagnosticWire, HookDispatchMessage, HookEffectWire};
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

    fn external_binding(id: &str) -> HookBinding {
        HookBinding {
            plugin_id: "external-test".to_string(),
            hook_id: id.to_string(),
            event: HookEventKey::ToolCall,
            dispatch_mode: HookDispatchMode::Cancellable,
            failure_policy: HookFailurePolicy::FailClosed,
            priority: 0,
            executor: HookExecutorRef::External("remote-tool-policy".to_string()),
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

    #[tokio::test]
    async fn external_dispatcher_receives_protocol_message_and_maps_effects() {
        let registry = BuiltinHookRegistry::new();
        let binding = external_binding("external-policy");
        let mut dispatcher = CapturingExternalDispatcher::default();

        let effects = dispatch_hooks_with_external(
            HookEventKey::ToolCall,
            tool_call_payload("call-1"),
            HookContext::new(),
            &[binding],
            &registry,
            Some(&mut dispatcher),
        )
        .await
        .expect("external hook should dispatch");

        let message = dispatcher
            .message
            .expect("external dispatcher should capture protocol message");
        assert_eq!(message.plugin_id, "external-test");
        assert_eq!(message.hook_id, "remote-tool-policy");
        assert_eq!(message.event, "tool_call");
        assert_eq!(message.payload["toolCallId"], "call-1");
        assert!(matches!(effects[0], HookEffect::Diagnostic { .. }));
        assert!(matches!(
            effects[1],
            HookEffect::BlockToolResult { ref tool_call_id, .. } if tool_call_id == "call-1"
        ));
    }

    #[derive(Default)]
    struct CapturingExternalDispatcher {
        message: Option<HookDispatchMessage>,
    }

    #[async_trait]
    impl ExternalHookDispatcher for CapturingExternalDispatcher {
        async fn dispatch_hook(
            &mut self,
            request: ExternalHookDispatchRequest,
        ) -> Result<Vec<HookEffect>> {
            let message = request.into_protocol_message()?;
            self.message = Some(message);
            hook_effects_from_wire(
                vec![HookEffectWire {
                    kind: "BlockToolResult".to_string(),
                    payload: serde_json::json!({
                        "toolCallId": "call-1",
                        "reason": "remote denied"
                    }),
                }],
                vec![HookDiagnosticWire {
                    message: "remote policy evaluated".to_string(),
                    severity: Some("info".to_string()),
                }],
            )
        }
    }
}
