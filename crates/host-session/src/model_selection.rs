use std::sync::Arc;

use astrcode_core::{AstrError, HookEventKey, ModelSelection, Result};
use astrcode_runtime_contract::hooks::{HookEffect, HookEventPayload};

use crate::ports::HookDispatch;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSelectionDecision {
    pub selection: ModelSelection,
    pub diagnostics: Vec<String>,
}

/// 使用 typed hook dispatch 执行 model_select hooks。
pub async fn apply_model_select_hooks(
    selection: ModelSelection,
    dispatcher: Option<Arc<dyn HookDispatch>>,
) -> Result<ModelSelectionDecision> {
    let Some(dispatcher) = dispatcher else {
        return Ok(ModelSelectionDecision {
            selection,
            diagnostics: Vec::new(),
        });
    };

    let payload = HookEventPayload::ModelSelect {
        session_id: String::new(),
        current_model: selection.model.clone(),
        candidate_model: selection.model.clone(),
        reason: format!("profile:{}", selection.profile_name),
    };
    let effects = dispatcher
        .dispatch_hook(HookEventKey::ModelSelect, payload)
        .await?;

    let mut diagnostics = Vec::new();
    let mut selection = selection;
    for effect in &effects {
        match effect {
            HookEffect::Diagnostic { message } => {
                diagnostics.push(message.clone());
            },
            HookEffect::ModelHint { model } => {
                selection.model = model.clone();
            },
            HookEffect::DenyModelSelect { reason } => {
                return Err(AstrError::Validation(format!(
                    "model_select denied by hook: {reason}"
                )));
            },
            _ => {},
        }
    }

    Ok(ModelSelectionDecision {
        selection,
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use astrcode_core::{HookEventKey, ModelSelection, Result};
    use astrcode_runtime_contract::hooks::{HookEffect, HookEventPayload};
    use async_trait::async_trait;

    use super::{ModelSelectionDecision, apply_model_select_hooks};
    use crate::ports::HookDispatch;

    fn selection() -> ModelSelection {
        ModelSelection::new("coding", "gpt-5.4", "openai")
    }

    struct DiagnosticHookDispatcher {
        message: String,
    }

    #[async_trait]
    impl HookDispatch for DiagnosticHookDispatcher {
        async fn dispatch_hook(
            &self,
            _event: HookEventKey,
            _payload: HookEventPayload,
        ) -> Result<Vec<HookEffect>> {
            Ok(vec![HookEffect::Diagnostic {
                message: self.message.clone(),
            }])
        }
    }

    struct DenyHookDispatcher;

    #[async_trait]
    impl HookDispatch for DenyHookDispatcher {
        async fn dispatch_hook(
            &self,
            _event: HookEventKey,
            _payload: HookEventPayload,
        ) -> Result<Vec<HookEffect>> {
            Ok(vec![HookEffect::DenyModelSelect {
                reason: "policy denied".to_string(),
            }])
        }
    }

    struct HintHookDispatcher;

    #[async_trait]
    impl HookDispatch for HintHookDispatcher {
        async fn dispatch_hook(
            &self,
            _event: HookEventKey,
            _payload: HookEventPayload,
        ) -> Result<Vec<HookEffect>> {
            Ok(vec![HookEffect::ModelHint {
                model: "deepseek-chat".to_string(),
            }])
        }
    }

    #[tokio::test]
    async fn model_select_keeps_selection_when_hooks_only_report_diagnostics() {
        let dispatcher = std::sync::Arc::new(DiagnosticHookDispatcher {
            message: "keep current model".to_string(),
        });
        let decision = apply_model_select_hooks(selection(), Some(dispatcher))
            .await
            .expect("diagnostic hook should not block selection");

        assert_eq!(
            decision,
            ModelSelectionDecision {
                selection: selection(),
                diagnostics: vec!["keep current model".to_string()],
            }
        );
    }

    #[tokio::test]
    async fn model_select_can_be_blocked_by_host_hook() {
        let dispatcher = std::sync::Arc::new(DenyHookDispatcher);
        let error = apply_model_select_hooks(selection(), Some(dispatcher))
            .await
            .expect_err("blocking hook should reject selection");

        assert!(error.to_string().contains("model_select denied by hook"));
    }

    #[tokio::test]
    async fn model_select_hint_rewrites_candidate_model() {
        let dispatcher = std::sync::Arc::new(HintHookDispatcher);
        let decision = apply_model_select_hooks(selection(), Some(dispatcher))
            .await
            .expect("model hint should succeed");

        assert_eq!(
            decision.selection,
            ModelSelection::new("coding", "deepseek-chat", "openai")
        );
    }

    #[tokio::test]
    async fn model_select_without_dispatcher_returns_plain_selection() {
        let decision = apply_model_select_hooks(selection(), None)
            .await
            .expect("no dispatcher should succeed");

        assert_eq!(decision.selection, selection());
        assert!(decision.diagnostics.is_empty());
    }
}
