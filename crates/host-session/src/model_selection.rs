use astrcode_core::{AstrError, HookEventKey, ModelSelection, Result};
use astrcode_plugin_host::{HookBusEffectKind, HookBusRequest, HookBusStep, dispatch_hook_bus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSelectionDecision {
    pub selection: ModelSelection,
    pub diagnostics: Vec<String>,
}

pub fn apply_model_select_hooks(
    selection: ModelSelection,
    steps: Vec<HookBusStep>,
) -> Result<ModelSelectionDecision> {
    let payload = serde_json::to_value(&selection).map_err(|error| {
        AstrError::Internal(format!("serialize model selection failed: {error}"))
    })?;
    let outcome = dispatch_hook_bus(
        HookBusRequest {
            event: HookEventKey::ModelSelect,
            payload,
        },
        steps,
    )?;

    if let Some(blocked_by) = outcome.blocked_by {
        return Err(AstrError::Validation(format!(
            "model_select blocked by hook '{blocked_by}'"
        )));
    }

    let selection = serde_json::from_value(outcome.payload)
        .map_err(|error| AstrError::Internal(format!("decode model selection failed: {error}")))?;
    let diagnostics = outcome
        .effects
        .into_iter()
        .filter(|effect| effect.kind == HookBusEffectKind::Diagnostic)
        .filter_map(|effect| effect.payload.as_str().map(str::to_owned))
        .collect();

    Ok(ModelSelectionDecision {
        selection,
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use astrcode_core::ModelSelection;
    use astrcode_plugin_host::{
        HookBusEffect, HookBusEffectKind, HookBusStep, HookDispatchMode, HookFailurePolicy,
        HookRegistration, HookStage,
    };
    use serde_json::json;

    use super::{ModelSelectionDecision, apply_model_select_hooks};

    fn selection() -> ModelSelection {
        ModelSelection::new("coding", "gpt-5.4", "openai")
    }

    fn step(hook_id: &str, dispatch_mode: HookDispatchMode, effect: HookBusEffect) -> HookBusStep {
        HookBusStep {
            registration: HookRegistration {
                descriptor: astrcode_plugin_host::HookDescriptor {
                    hook_id: hook_id.to_string(),
                    event: "model_select".to_string(),
                },
                stage: HookStage::Host,
                dispatch_mode,
                failure_policy: HookFailurePolicy::FailClosed,
                priority: 0,
            },
            effect,
        }
    }

    #[test]
    fn model_select_keeps_selection_when_hooks_only_report_diagnostics() {
        let decision = apply_model_select_hooks(
            selection(),
            vec![step(
                "diag",
                HookDispatchMode::Sequential,
                HookBusEffect {
                    kind: HookBusEffectKind::Diagnostic,
                    payload: json!("keep current model"),
                    terminal: false,
                },
            )],
        )
        .expect("diagnostic hook should not block selection");

        assert_eq!(
            decision,
            ModelSelectionDecision {
                selection: selection(),
                diagnostics: vec!["keep current model".to_string()],
            }
        );
    }

    #[test]
    fn model_select_can_rewrite_selection_payload() {
        let decision = apply_model_select_hooks(
            selection(),
            vec![step(
                "rewrite",
                HookDispatchMode::Modify,
                HookBusEffect {
                    kind: HookBusEffectKind::MutatePayload,
                    payload: json!({
                        "profileName": "coding",
                        "model": "deepseek-chat",
                        "providerKind": "deepseek"
                    }),
                    terminal: false,
                },
            )],
        )
        .expect("rewrite hook should produce a new selection");

        assert_eq!(
            decision.selection,
            ModelSelection::new("coding", "deepseek-chat", "deepseek")
        );
    }

    #[test]
    fn model_select_can_be_blocked_by_host_hook() {
        let error = apply_model_select_hooks(
            selection(),
            vec![step(
                "policy",
                HookDispatchMode::Cancellable,
                HookBusEffect {
                    kind: HookBusEffectKind::Block,
                    payload: json!({ "reason": "policy denied" }),
                    terminal: true,
                },
            )],
        )
        .expect_err("blocking hook should reject selection");

        assert!(
            error
                .to_string()
                .contains("model_select blocked by hook 'policy'")
        );
    }
}
