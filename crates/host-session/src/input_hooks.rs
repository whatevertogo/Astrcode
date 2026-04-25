use std::sync::Arc;

use astrcode_core::{AstrError, HookEventKey, Result, SessionId, mode::ModeId};
use astrcode_runtime_contract::hooks::{HookEffect, HookEventPayload};

use crate::ports::HookDispatch;

/// host-session input hook 的输入。
#[derive(Clone)]
pub struct InputHookApplyRequest {
    pub session_id: SessionId,
    pub source: String,
    pub text: String,
    pub images: Vec<String>,
    pub current_mode: ModeId,
    pub dispatcher: Option<Arc<dyn HookDispatch>>,
}

/// input hook 应用后的 owner 决策。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputHookDecision {
    Continue {
        text: String,
        switch_mode: Option<ModeId>,
    },
    Handled {
        session_id: SessionId,
        response: String,
        switch_mode: Option<ModeId>,
    },
}

/// 在 turn acceptance 前应用 `input` hook effects。
pub async fn apply_input_hooks(request: InputHookApplyRequest) -> Result<InputHookDecision> {
    let Some(dispatcher) = request.dispatcher else {
        return Ok(InputHookDecision::Continue {
            text: request.text,
            switch_mode: None,
        });
    };

    let payload = HookEventPayload::Input {
        session_id: request.session_id.to_string(),
        source: request.source,
        text: request.text.clone(),
        images: request.images,
        current_mode: Some(request.current_mode.to_string()),
    };
    let effects = dispatcher
        .dispatch_hook(HookEventKey::Input, payload)
        .await?;

    let mut text = request.text;
    let mut switch_mode = None;
    let mut handled_response = None;
    for effect in effects {
        match effect {
            HookEffect::Continue | HookEffect::Diagnostic { .. } => {},
            HookEffect::TransformInput {
                text: transformed_text,
            } => {
                text = transformed_text;
            },
            HookEffect::HandledInput { response } => {
                handled_response = Some(response);
            },
            HookEffect::SwitchMode { mode_id } => {
                switch_mode = Some(ModeId::new(mode_id)?);
            },
            other => {
                return Err(AstrError::Validation(format!(
                    "input hook returned unsupported effect '{}'",
                    hook_effect_name(&other)
                )));
            },
        }
    }

    if let Some(response) = handled_response {
        return Ok(InputHookDecision::Handled {
            session_id: request.session_id,
            response,
            switch_mode,
        });
    }

    Ok(InputHookDecision::Continue { text, switch_mode })
}

fn hook_effect_name(effect: &HookEffect) -> &'static str {
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

#[cfg(test)]
mod tests {
    use astrcode_core::{HookEventKey, Result, SessionId, mode::ModeId};
    use astrcode_runtime_contract::hooks::{HookEffect, HookEventPayload};
    use async_trait::async_trait;

    use super::{InputHookApplyRequest, InputHookDecision, apply_input_hooks};
    use crate::ports::HookDispatch;

    struct EffectsDispatcher {
        effects: Vec<HookEffect>,
    }

    #[async_trait]
    impl HookDispatch for EffectsDispatcher {
        async fn dispatch_hook(
            &self,
            event: HookEventKey,
            payload: HookEventPayload,
        ) -> Result<Vec<HookEffect>> {
            assert_eq!(event, HookEventKey::Input);
            assert!(matches!(payload, HookEventPayload::Input { .. }));
            Ok(self.effects.clone())
        }
    }

    fn request(effects: Vec<HookEffect>) -> InputHookApplyRequest {
        InputHookApplyRequest {
            session_id: SessionId::from("session-1"),
            source: "user".to_string(),
            text: "draft this".to_string(),
            images: Vec::new(),
            current_mode: ModeId::code(),
            dispatcher: Some(std::sync::Arc::new(EffectsDispatcher { effects })),
        }
    }

    #[tokio::test]
    async fn input_hook_transforms_text_before_turn_acceptance() {
        let decision = apply_input_hooks(request(vec![HookEffect::TransformInput {
            text: "transformed".to_string(),
        }]))
        .await
        .expect("input hook should apply");

        assert_eq!(
            decision,
            InputHookDecision::Continue {
                text: "transformed".to_string(),
                switch_mode: None,
            }
        );
    }

    #[tokio::test]
    async fn input_hook_can_handle_input_without_creating_turn() {
        let decision = apply_input_hooks(request(vec![HookEffect::HandledInput {
            response: "handled".to_string(),
        }]))
        .await
        .expect("input hook should apply");

        assert_eq!(
            decision,
            InputHookDecision::Handled {
                session_id: SessionId::from("session-1"),
                response: "handled".to_string(),
                switch_mode: None,
            }
        );
    }

    #[tokio::test]
    async fn input_hook_can_request_mode_switch() {
        let decision = apply_input_hooks(request(vec![HookEffect::SwitchMode {
            mode_id: "plan".to_string(),
        }]))
        .await
        .expect("input hook should apply");

        assert_eq!(
            decision,
            InputHookDecision::Continue {
                text: "draft this".to_string(),
                switch_mode: Some(ModeId::plan()),
            }
        );
    }
}
