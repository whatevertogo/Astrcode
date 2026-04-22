//! # 策略引擎
//!
//! 定义了策略引擎的抽象接口和类型，用于控制 Agent 的行为。
//!
//! ## 核心职责
//!
//! - **审批流程**: 决定某个能力调用是否需要用户审批
//! - **内容审查**: 检查/修改 LLM 请求和工具调用
//! - **模型/工具护栏**: 为运行时提供统一的审批与请求检查入口
//!
//! ## 裁决流程
//!
//! 每次能力调用都会经过 `check_capability_call`，返回三种裁决之一：
//! `Allow`（直接执行）、`Deny`（拒绝并说明原因）、`Ask`（等待用户审批）。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{CapabilitySpec, LlmMessage, Result, ToolDefinition};

/// 系统提示词块所属层级。
///
/// provider 可以利用该层级决定缓存边界，从而在分层 prompt 下尽量保住稳定前缀。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SystemPromptLayer {
    Stable,
    SemiStable,
    Inherited,
    Dynamic,
    #[default]
    Unspecified,
}

/// 已渲染的系统提示词块。
///
/// RequestAssembler 会把 `PromptPlan` 中的 system blocks 降级为这个 provider 无关 DTO。
/// 这样 `core` 只感知“分段后的系统提示词”，而不依赖 `adapter-prompt` 的内部类型。
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
    /// 消息历史
    pub messages: Vec<LlmMessage>,
    /// 可用工具列表
    pub tools: Vec<ToolDefinition>,
    /// 系统提示词
    pub system_prompt: Option<String>,
    /// 分段后的系统提示词块。
    ///
    /// 默认 provider 可忽略它继续使用 `system_prompt`，支持分层缓存或稳定前缀优化的
    /// 后端则可以直接消费它。
    pub system_prompt_blocks: Vec<SystemPromptBlock>,
}

/// 通用能力调用提案
///
/// 流经策略层等待执行的通用能力调用提案。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityCall {
    /// 请求 ID
    pub request_id: String,
    /// 能力规范
    pub capability: CapabilitySpec,
    /// 调用载荷
    pub payload: Value,
    /// 元数据
    #[serde(default)]
    pub metadata: Value,
}

impl CapabilityCall {
    /// 获取能力名称
    pub fn name(&self) -> &str {
        self.capability.name.as_str()
    }
}

/// 策略上下文
///
/// 策略实现可用的共享 Turn 元数据，不暴露传输细节。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyContext {
    /// 会话 ID
    pub session_id: String,
    /// Turn ID
    pub turn_id: String,
    /// 步骤索引
    pub step_index: usize,
    /// 工作目录
    pub working_dir: String,
    /// Profile 名称
    pub profile: String,
    /// 元数据
    #[serde(default)]
    pub metadata: Value,
}

/// 审批默认值
///
/// 用于 UI 展示默认选项。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDefault {
    /// 默认允许
    Allow,
    /// 默认拒绝
    Deny,
}

impl ApprovalDefault {
    /// 解析为审批结果
    pub fn resolve(self) -> ApprovalResolution {
        match self {
            Self::Allow => ApprovalResolution::approved(),
            Self::Deny => ApprovalResolution::denied("approval denied by default"),
        }
    }
}

/// 审批请求
///
/// 策略层产生的、需要用户确认的审批载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    /// 请求 ID
    pub request_id: String,
    /// 会话 ID
    pub session_id: String,
    /// Turn ID
    pub turn_id: String,
    /// 能力描述符
    pub capability: CapabilitySpec,
    /// 调用载荷
    pub payload: Value,
    /// 提示文本（向用户展示）
    pub prompt: String,
    /// 默认选项
    pub default: ApprovalDefault,
    /// 元数据
    #[serde(default)]
    pub metadata: Value,
}

impl ApprovalRequest {
    /// 获取能力名称
    pub fn capability_name(&self) -> &str {
        &self.capability.name
    }
}

/// 审批结果
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalResolution {
    /// 是否批准
    pub approved: bool,
    /// 拒绝原因（批准时为 None）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ApprovalResolution {
    /// 创建批准结果
    pub fn approved() -> Self {
        Self {
            approved: true,
            reason: None,
        }
    }

    /// 创建拒绝结果
    pub fn denied(reason: impl Into<String>) -> Self {
        Self {
            approved: false,
            reason: Some(reason.into()),
        }
    }
}

/// 等待审批的动作
///
/// 包装审批请求和待执行的动作。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalPending<T> {
    /// 审批请求
    pub request: ApprovalRequest,
    /// 审批通过后要执行的动作
    pub action: T,
}

/// 策略裁决
///
/// 策略引擎对能力调用的裁决结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyVerdict<T> {
    /// 允许执行
    Allow(T),
    /// 拒绝执行
    Deny { reason: String },
    /// 需要用户审批
    ///
    /// Box 住审批载荷，避免 Allow/Deny 也为较大的审批上下文付出同样的栈空间成本。
    Ask(Box<ApprovalPending<T>>),
}

impl<T> PolicyVerdict<T> {
    /// 创建允许裁决
    pub fn allow(value: T) -> Self {
        Self::Allow(value)
    }

    /// 创建拒绝裁决
    pub fn deny(reason: impl Into<String>) -> Self {
        Self::Deny {
            reason: reason.into(),
        }
    }

    /// 创建需审批裁决
    pub fn ask(request: ApprovalRequest, action: T) -> Self {
        Self::Ask(Box::new(ApprovalPending { request, action }))
    }
}

/// 策略引擎 trait。
///
/// 定义了策略引擎必须实现的两个核心检查点：
/// 1. `check_model_request`: 在发送 LLM 请求前，可检查/重写请求内容
/// 2. `check_capability_call`: 在执行能力调用前，决定允许/拒绝/需审批
#[async_trait]
pub trait PolicyEngine: Send + Sync {
    /// 检查/重写模型请求。
    ///
    /// 策略实现可以修改消息、工具列表或系统提示词，
    /// 用于内容审查、敏感信息过滤等场景。
    async fn check_model_request(
        &self,
        request: ModelRequest,
        ctx: &PolicyContext,
    ) -> Result<ModelRequest>;

    /// 检查能力调用是否需要审批。
    ///
    /// 返回 `Allow`（直接执行）、`Deny`（拒绝）或 `Ask`（等待用户审批）。
    async fn check_capability_call(
        &self,
        call: CapabilityCall,
        ctx: &PolicyContext,
    ) -> Result<PolicyVerdict<CapabilityCall>>;
}

/// 允许所有操作的策略引擎。
///
/// 用于测试和不启用策略的场景，所有请求和调用都直接放行。
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
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{
        AllowAllPolicyEngine, ApprovalDefault, ApprovalRequest, PolicyContext, PolicyEngine,
        PolicyVerdict,
    };
    use crate::{
        CapabilityKind, CapabilitySpec, InvocationMode, ModelRequest, SideEffect, Stability,
    };

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
    async fn allow_all_policy_preserves_requests() {
        let policy = AllowAllPolicyEngine;
        let request = ModelRequest {
            messages: vec![],
            tools: vec![],
            system_prompt: Some("system".to_string()),
            system_prompt_blocks: Vec::new(),
        };
        let call = super::CapabilityCall {
            request_id: "call-1".to_string(),
            capability: capability("tool.sample"),
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
            capability: capability("tool.sample"),
            payload: json!({}),
            prompt: "Allow?".to_string(),
            default: ApprovalDefault::Deny,
            metadata: Value::Null,
        };

        assert_eq!(request.capability_name(), "tool.sample");
    }
}
