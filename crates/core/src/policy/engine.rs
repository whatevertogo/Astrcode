//! # 策略引擎
//!
//! 定义了策略引擎的抽象接口和类型，用于控制 Agent 的行为。
//!
//! ## 核心职责
//!
//! - **审批流程**: 决定某个能力调用是否需要用户审批
//! - **内容审查**: 检查/修改 LLM 请求和工具调用
//! - **上下文策略**: 当上下文压力过大时决定压缩策略

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{CapabilityDescriptor, LlmMessage, Result, ToolDefinition};

/// Turn 范围的模型请求
///
/// 策略引擎可以检查或重写此请求，用于内容审查、敏感信息过滤等。
#[derive(Debug, Clone)]
/// Turn 范围的模型请求，策略层可在执行前检查或重写。
pub struct ModelRequest {
    /// 消息历史
    pub messages: Vec<LlmMessage>,
    /// 可用工具列表
    pub tools: Vec<ToolDefinition>,
    /// 系统提示词
    pub system_prompt: Option<String>,
}

/// 通用能力调用提案
///
/// 流经策略层等待执行的通用能力调用提案。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityCall {
    /// 请求 ID
    pub request_id: String,
    /// 能力描述符
    pub descriptor: CapabilityDescriptor,
    /// 调用载荷
    pub payload: Value,
    /// 元数据
    #[serde(default)]
    pub metadata: Value,
}

impl CapabilityCall {
    /// 获取能力名称
    pub fn name(&self) -> &str {
        &self.descriptor.name
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
    pub capability: CapabilityDescriptor,
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
    Ask(ApprovalPending<T>),
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
        Self::Ask(ApprovalPending { request, action })
    }
}

/// 上下文压力输入
///
/// 用于决策如何处理过长的上下文。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextPressureInput {
    /// 已使用 token 数
    pub used_tokens: u32,
    /// Token 上限
    pub limit_tokens: u32,
}

/// 上下文策略决策
///
/// 当上下文过长时，策略引擎决定如何处理。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContextStrategyDecision {
    /// 紧凑化（移除冗余）
    Compact,
    /// 摘要化（将旧消息摘要）
    Summarize,
    /// 截断（丢弃旧消息）
    Truncate,
    /// 忽略（不做处理）
    #[default]
    Ignore,
}

/// 策略引擎 trait
///
/// 定义了策略引擎必须实现的接口。
#[async_trait]
pub trait PolicyEngine: Send + Sync {
    /// 检查/重写模型请求
    ///
    /// 策略引擎可以修改请求内容，用于内容审查等。
    async fn check_model_request(
        &self,
        request: ModelRequest,
        ctx: &PolicyContext,
    ) -> Result<ModelRequest>;

    /// 检查能力调用是否需要审批
    ///
    /// 返回允许、拒绝或需用户审批。
    async fn check_capability_call(
        &self,
        call: CapabilityCall,
        ctx: &PolicyContext,
    ) -> Result<PolicyVerdict<CapabilityCall>>;

    /// 决策上下文策略
    ///
    /// 当上下文压力过大时，决定如何处理。
    async fn decide_context_strategy(
        &self,
        input: ContextPressureInput,
        ctx: &PolicyContext,
    ) -> Result<ContextStrategyDecision>;
}

/// 允许所有操作的策略引擎
///
/// 用于测试和不策略的场景。
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
