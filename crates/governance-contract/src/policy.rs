//! # 策略引擎
//!
//! 定义治理层的策略接口和请求/审批类型。

use astrcode_core::{
    CapabilitySpec, LlmMessage, action::ToolDefinition, policy::SystemPromptLayer,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 已渲染的系统提示词块。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SystemPromptBlock {
    pub title: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub cache_boundary: bool,
    #[serde(default, skip_serializing_if = "is_unspecified_system_prompt_layer")]
    pub layer: SystemPromptLayer,
}

impl SystemPromptBlock {
    pub fn render(&self) -> String {
        format!("[{}]\n{}", self.title, self.content)
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_unspecified_system_prompt_layer(layer: &SystemPromptLayer) -> bool {
    matches!(layer, SystemPromptLayer::Unspecified)
}

/// Turn 范围的模型请求，策略层可在执行前检查或重写。
#[derive(Debug, Clone)]
pub struct ModelRequest {
    pub messages: Vec<LlmMessage>,
    pub tools: Vec<ToolDefinition>,
    pub system_prompt: Option<String>,
    pub system_prompt_blocks: Vec<SystemPromptBlock>,
}

/// 通用能力调用提案。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityCall {
    pub request_id: String,
    pub capability: CapabilitySpec,
    pub payload: Value,
    #[serde(default)]
    pub metadata: Value,
}

impl CapabilityCall {
    pub fn name(&self) -> &str {
        self.capability.name.as_str()
    }
}

/// 策略实现可用的共享 turn 元数据，不暴露传输细节。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyContext {
    pub session_id: String,
    pub turn_id: String,
    pub step_index: usize,
    pub working_dir: String,
    pub profile: String,
    #[serde(default)]
    pub metadata: Value,
}

/// 审批默认值。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDefault {
    Allow,
    Deny,
}

/// 策略层产生的、需要用户确认的审批载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub request_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub capability: CapabilitySpec,
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

/// 等待审批的动作。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalPending<T> {
    pub request: ApprovalRequest,
    pub action: T,
}

#[cfg(test)]
mod tests {
    use astrcode_core::{CapabilityKind, CapabilitySpec, InvocationMode, SideEffect, Stability};
    use serde_json::{Value, json};

    use super::{ApprovalDefault, ApprovalRequest};

    fn capability(name: &str) -> CapabilitySpec {
        CapabilitySpec {
            name: name.into(),
            kind: CapabilityKind::Tool,
            description: "test capability".to_string(),
            input_schema: json!({ "type": "object" }),
            output_schema: json!({ "type": "object" }),
            invocation_mode: InvocationMode::Unary,
            concurrency_safe: false,
            compact_clearable: false,
            profiles: vec!["coding".to_string()],
            tags: vec![],
            permissions: vec![],
            side_effect: SideEffect::Workspace,
            stability: Stability::Stable,
            metadata: Value::Null,
            max_result_inline_size: None,
        }
    }

    #[test]
    fn approval_request_reports_capability_name() {
        let request = ApprovalRequest {
            request_id: "call-1".to_string(),
            session_id: "session-1".to_string(),
            turn_id: "turn-1".to_string(),
            capability: capability("tool.sample"),
            payload: json!({}),
            prompt: "Allow?".to_string(),
            default: ApprovalDefault::Deny,
            metadata: Value::Null,
        };

        assert_eq!(request.capability_name(), "tool.sample");
    }
}
