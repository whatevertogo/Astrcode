use async_trait::async_trait;

use astrcode_core::{ApprovalRequest, ApprovalResolution, CancelToken, Result};

#[async_trait]
pub trait ApprovalBroker: Send + Sync {
    /// Resolves a policy-generated approval request.
    ///
    /// The broker is deliberately transport-agnostic. A CLI, Web UI, or ACP bridge can sit behind
    /// this trait later without changing the agent loop contract.
    async fn request(
        &self,
        request: ApprovalRequest,
        cancel: CancelToken,
    ) -> Result<ApprovalResolution>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultApprovalBroker;

#[async_trait]
impl ApprovalBroker for DefaultApprovalBroker {
    async fn request(
        &self,
        request: ApprovalRequest,
        _cancel: CancelToken,
    ) -> Result<ApprovalResolution> {
        Ok(request.default.resolve())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ApprovalBroker, DefaultApprovalBroker};
    use astrcode_core::{
        ApprovalDefault, ApprovalRequest, CancelToken, CapabilityDescriptor, CapabilityKind,
        SideEffectLevel, StabilityLevel,
    };

    fn request(default: ApprovalDefault) -> ApprovalRequest {
        ApprovalRequest {
            request_id: "call-1".to_string(),
            session_id: "session-1".to_string(),
            turn_id: "turn-1".to_string(),
            capability: CapabilityDescriptor {
                name: "tool.sample".to_string(),
                kind: CapabilityKind::tool(),
                description: "sample".to_string(),
                input_schema: json!({ "type": "object" }),
                output_schema: json!({ "type": "object" }),
                streaming: false,
                profiles: vec!["coding".to_string()],
                tags: vec![],
                permissions: vec![],
                side_effect: SideEffectLevel::Workspace,
                stability: StabilityLevel::Stable,
            },
            payload: json!({}),
            prompt: "Allow sample?".to_string(),
            default,
            metadata: json!({ "source": "test" }),
        }
    }

    #[tokio::test]
    async fn default_broker_resolves_using_request_default() {
        let broker = DefaultApprovalBroker;

        assert!(
            broker
                .request(request(ApprovalDefault::Allow), CancelToken::new())
                .await
                .expect("allow resolution should succeed")
                .approved
        );

        assert!(
            !broker
                .request(request(ApprovalDefault::Deny), CancelToken::new())
                .await
                .expect("deny resolution should succeed")
                .approved
        );
    }
}
