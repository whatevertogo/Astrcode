use astrcode_core::{AstrError, HookEventKey, Result};
use serde_json::Value;

use crate::HookDescriptor;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookBusEffectKind {
    Continue,
    Block,
    CancelTurn,
    TransformInput,
    AugmentPrompt,
    MutatePayload,
    OverrideToolResult,
    ResourcePath,
    ModelHint,
    Diagnostic,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HookBusEffect {
    pub kind: HookBusEffectKind,
    pub payload: Value,
    pub terminal: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookRegistration {
    pub descriptor: HookDescriptor,
    pub stage: HookStage,
    pub dispatch_mode: HookDispatchMode,
    pub failure_policy: HookFailurePolicy,
    pub priority: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HookBusStep {
    pub registration: HookRegistration,
    pub effect: HookBusEffect,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HookBusRequest {
    pub event: HookEventKey,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HookBusOutcome {
    pub payload: Value,
    pub effects: Vec<HookBusEffect>,
    pub blocked_by: Option<String>,
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

pub fn dispatch_hook_bus(
    request: HookBusRequest,
    mut steps: Vec<HookBusStep>,
) -> Result<HookBusOutcome> {
    if !SUPPORTED_HOOK_EVENTS.contains(&request.event) {
        return Err(AstrError::Validation(format!(
            "hook event '{:?}' is not supported by plugin-host hook bus",
            request.event
        )));
    }
    steps.sort_by(|left, right| {
        right
            .registration
            .priority
            .cmp(&left.registration.priority)
            .then_with(|| {
                left.registration
                    .descriptor
                    .hook_id
                    .cmp(&right.registration.descriptor.hook_id)
            })
    });

    let mut payload = request.payload;
    let mut effects = Vec::new();
    let mut blocked_by = None;

    for step in steps {
        if !hook_targets_event(&step.registration.descriptor, request.event) {
            continue;
        }

        let effect = step.effect;
        match step.registration.dispatch_mode {
            HookDispatchMode::Modify | HookDispatchMode::Pipeline => {
                if matches!(
                    effect.kind,
                    HookBusEffectKind::TransformInput | HookBusEffectKind::MutatePayload
                ) {
                    payload = effect.payload.clone();
                }
            },
            HookDispatchMode::Cancellable
            | HookDispatchMode::Intercept
            | HookDispatchMode::ShortCircuit => {
                if effect.terminal
                    || matches!(
                        effect.kind,
                        HookBusEffectKind::Block | HookBusEffectKind::CancelTurn
                    )
                {
                    blocked_by = Some(step.registration.descriptor.hook_id.clone());
                    effects.push(effect);
                    break;
                }
            },
            HookDispatchMode::Sequential => {},
        }
        effects.push(effect);
    }

    Ok(HookBusOutcome {
        payload,
        effects,
        blocked_by,
    })
}

fn hook_targets_event(descriptor: &HookDescriptor, event: HookEventKey) -> bool {
    descriptor.event == format!("{event:?}") || descriptor.event == hook_event_key_name(event)
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
    use serde_json::json;

    use super::*;

    fn registration(
        id: &str,
        event: &str,
        priority: i32,
        dispatch_mode: HookDispatchMode,
    ) -> HookRegistration {
        HookRegistration {
            descriptor: HookDescriptor {
                hook_id: id.to_string(),
                event: event.to_string(),
            },
            stage: HookStage::Runtime,
            dispatch_mode,
            failure_policy: HookFailurePolicy::FailClosed,
            priority,
        }
    }

    fn effect(kind: HookBusEffectKind, payload: Value, terminal: bool) -> HookBusEffect {
        HookBusEffect {
            kind,
            payload,
            terminal,
        }
    }

    #[test]
    fn hook_bus_lists_the_full_event_surface() {
        assert_eq!(SUPPORTED_HOOK_EVENTS.len(), 11);
        assert!(SUPPORTED_HOOK_EVENTS.contains(&HookEventKey::ResourcesDiscover));
        assert!(SUPPORTED_HOOK_EVENTS.contains(&HookEventKey::ModelSelect));
    }

    #[test]
    fn hook_bus_runs_matching_hooks_by_priority_order() {
        let outcome = dispatch_hook_bus(
            HookBusRequest {
                event: HookEventKey::ToolCall,
                payload: json!({ "tool": "readFile" }),
            },
            vec![
                HookBusStep {
                    registration: registration("low", "tool_call", 1, HookDispatchMode::Sequential),
                    effect: effect(HookBusEffectKind::Diagnostic, json!("low"), false),
                },
                HookBusStep {
                    registration: registration(
                        "high",
                        "tool_call",
                        10,
                        HookDispatchMode::Sequential,
                    ),
                    effect: effect(HookBusEffectKind::Diagnostic, json!("high"), false),
                },
            ],
        )
        .expect("hook bus should dispatch");

        assert_eq!(outcome.effects[0].payload, json!("high"));
        assert_eq!(outcome.effects[1].payload, json!("low"));
        assert!(outcome.blocked_by.is_none());
    }

    #[test]
    fn hook_bus_stops_on_terminal_blocking_effect() {
        let outcome = dispatch_hook_bus(
            HookBusRequest {
                event: HookEventKey::BeforeProviderRequest,
                payload: json!({ "model": "gpt" }),
            },
            vec![
                HookBusStep {
                    registration: registration(
                        "blocker",
                        "before_provider_request",
                        10,
                        HookDispatchMode::Cancellable,
                    ),
                    effect: effect(
                        HookBusEffectKind::Block,
                        json!({ "reason": "policy" }),
                        true,
                    ),
                },
                HookBusStep {
                    registration: registration(
                        "later",
                        "before_provider_request",
                        1,
                        HookDispatchMode::Sequential,
                    ),
                    effect: effect(
                        HookBusEffectKind::Diagnostic,
                        json!("should-not-run"),
                        false,
                    ),
                },
            ],
        )
        .expect("hook bus should dispatch");

        assert_eq!(outcome.blocked_by.as_deref(), Some("blocker"));
        assert_eq!(outcome.effects.len(), 1);
    }

    #[test]
    fn hook_bus_applies_modify_and_pipeline_payloads() {
        let outcome = dispatch_hook_bus(
            HookBusRequest {
                event: HookEventKey::Input,
                payload: json!({ "text": "original" }),
            },
            vec![
                HookBusStep {
                    registration: registration("rewrite", "input", 1, HookDispatchMode::Modify),
                    effect: effect(
                        HookBusEffectKind::TransformInput,
                        json!({ "text": "rewritten" }),
                        false,
                    ),
                },
                HookBusStep {
                    registration: registration("pipeline", "input", 0, HookDispatchMode::Pipeline),
                    effect: effect(
                        HookBusEffectKind::MutatePayload,
                        json!({ "text": "pipelined" }),
                        false,
                    ),
                },
            ],
        )
        .expect("hook bus should dispatch");

        assert_eq!(outcome.payload, json!({ "text": "pipelined" }));
        assert_eq!(outcome.effects.len(), 2);
    }
}
