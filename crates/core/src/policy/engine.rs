use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{CapabilityDescriptor, LlmMessage, Result, ToolDefinition};

#[derive(Debug, Clone)]
/// Turn-scoped model request that the policy layer may inspect or rewrite before execution.
pub struct ModelRequest {
    pub messages: Vec<LlmMessage>,
    pub tools: Vec<ToolDefinition>,
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Generic capability invocation proposal flowing through the policy layer before execution.
pub struct CapabilityCall {
    pub request_id: String,
    pub descriptor: CapabilityDescriptor,
    pub payload: Value,
    #[serde(default)]
    pub metadata: Value,
}

impl CapabilityCall {
    pub fn name(&self) -> &str {
        &self.descriptor.name
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Shared turn metadata available to policy implementations without exposing transport details.
pub struct PolicyContext {
    pub session_id: String,
    pub turn_id: String,
    pub step_index: usize,
    pub working_dir: String,
    pub profile: String,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDefault {
    Allow,
    Deny,
}

impl ApprovalDefault {
    pub fn resolve(self) -> ApprovalResolution {
        match self {
            Self::Allow => ApprovalResolution::approved(),
            Self::Deny => ApprovalResolution::denied("approval denied by default"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
/// Broker-facing approval payload produced by the policy layer when execution must pause.
pub struct ApprovalRequest {
    pub request_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub capability: CapabilityDescriptor,
    pub payload: Value,
    pub prompt: String,
    pub default: ApprovalDefault,
    #[serde(default)]
    pub metadata: Value,
}

impl ApprovalRequest {
    pub fn capability_name(&self) -> &str {
        &self.capability.name
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalResolution {
    pub approved: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ApprovalResolution {
    pub fn approved() -> Self {
        Self {
            approved: true,
            reason: None,
        }
    }

    pub fn denied(reason: impl Into<String>) -> Self {
        Self {
            approved: false,
            reason: Some(reason.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalPending<T> {
    pub request: ApprovalRequest,
    pub action: T,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyVerdict<T> {
    Allow(T),
    Deny { reason: String },
    Ask(ApprovalPending<T>),
}

impl<T> PolicyVerdict<T> {
    pub fn allow(value: T) -> Self {
        Self::Allow(value)
    }

    pub fn deny(reason: impl Into<String>) -> Self {
        Self::Deny {
            reason: reason.into(),
        }
    }

    pub fn ask(request: ApprovalRequest, action: T) -> Self {
        Self::Ask(ApprovalPending { request, action })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextPressureInput {
    pub used_tokens: u32,
    pub limit_tokens: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContextStrategyDecision {
    Compact,
    Summarize,
    Truncate,
    #[default]
    Ignore,
}

#[async_trait]
pub trait PolicyEngine: Send + Sync {
    async fn check_model_request(
        &self,
        request: ModelRequest,
        ctx: &PolicyContext,
    ) -> Result<ModelRequest>;

    async fn check_capability_call(
        &self,
        call: CapabilityCall,
        ctx: &PolicyContext,
    ) -> Result<PolicyVerdict<CapabilityCall>>;

    async fn decide_context_strategy(
        &self,
        input: ContextPressureInput,
        ctx: &PolicyContext,
    ) -> Result<ContextStrategyDecision>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AllowAllPolicyEngine;

#[async_trait]
impl PolicyEngine for AllowAllPolicyEngine {
    async fn check_model_request(
        &self,
        request: ModelRequest,
        _ctx: &PolicyContext,
    ) -> Result<ModelRequest> {
        Ok(request)
    }

    async fn check_capability_call(
        &self,
        call: CapabilityCall,
        _ctx: &PolicyContext,
    ) -> Result<PolicyVerdict<CapabilityCall>> {
        Ok(PolicyVerdict::Allow(call))
    }

    async fn decide_context_strategy(
        &self,
        _input: ContextPressureInput,
        _ctx: &PolicyContext,
    ) -> Result<ContextStrategyDecision> {
        Ok(ContextStrategyDecision::Ignore)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use super::{
        AllowAllPolicyEngine, ApprovalDefault, ApprovalRequest, ContextPressureInput,
        ContextStrategyDecision, PolicyContext, PolicyEngine, PolicyVerdict,
    };
    use crate::{
        CapabilityDescriptor, CapabilityKind, ModelRequest, SideEffectLevel, StabilityLevel,
    };

    fn descriptor(name: &str) -> CapabilityDescriptor {
        CapabilityDescriptor {
            name: name.to_string(),
            kind: CapabilityKind::tool(),
            description: "test capability".to_string(),
            input_schema: json!({ "type": "object" }),
            output_schema: json!({ "type": "object" }),
            streaming: false,
            profiles: vec!["coding".to_string()],
            tags: vec![],
            permissions: vec![],
            side_effect: SideEffectLevel::Workspace,
            stability: StabilityLevel::Stable,
        }
    }

    fn policy_context() -> PolicyContext {
        PolicyContext {
            session_id: "session-1".to_string(),
            turn_id: "turn-1".to_string(),
            step_index: 0,
            working_dir: ".".to_string(),
            profile: "coding".to_string(),
            metadata: Value::Null,
        }
    }

    #[tokio::test]
    async fn allow_all_policy_preserves_requests_and_ignores_context_pressure() {
        let policy = AllowAllPolicyEngine;
        let request = ModelRequest {
            messages: vec![],
            tools: vec![],
            system_prompt: Some("system".to_string()),
        };
        let call = super::CapabilityCall {
            request_id: "call-1".to_string(),
            descriptor: descriptor("tool.sample"),
            payload: json!({ "path": "Cargo.toml" }),
            metadata: Value::Null,
        };

        let checked_request = policy
            .check_model_request(request, &policy_context())
            .await
            .expect("request should pass");
        assert_eq!(checked_request.system_prompt.as_deref(), Some("system"));
        assert!(checked_request.messages.is_empty());
        assert!(checked_request.tools.is_empty());
        assert_eq!(
            policy
                .check_capability_call(call.clone(), &policy_context())
                .await
                .expect("call should pass"),
            PolicyVerdict::Allow(call)
        );
        assert_eq!(
            policy
                .decide_context_strategy(
                    ContextPressureInput {
                        used_tokens: 10,
                        limit_tokens: 100,
                    },
                    &policy_context(),
                )
                .await
                .expect("context strategy should be returned"),
            ContextStrategyDecision::Ignore
        );
    }

    #[test]
    fn approval_default_resolves_to_expected_resolution() {
        assert!(ApprovalDefault::Allow.resolve().approved);
        assert_eq!(
            ApprovalDefault::Deny.resolve(),
            super::ApprovalResolution {
                approved: false,
                reason: Some("approval denied by default".to_string()),
            }
        );
    }

    #[test]
    fn approval_request_reports_capability_name() {
        let request = ApprovalRequest {
            request_id: "call-1".to_string(),
            session_id: "session-1".to_string(),
            turn_id: "turn-1".to_string(),
            capability: descriptor("tool.sample"),
            payload: json!({}),
            prompt: "Allow?".to_string(),
            default: ApprovalDefault::Deny,
            metadata: Value::Null,
        };

        assert_eq!(request.capability_name(), "tool.sample");
    }
}
