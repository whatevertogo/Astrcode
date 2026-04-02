//! # 审批代理
//!
//! 审批代理负责处理需要用户确认的能力调用。

use async_trait::async_trait;

use astrcode_core::{ApprovalRequest, ApprovalResolution, CancelToken, Result};

/// 审批代理 trait
///
/// 故意设计为传输无关，CLI、Web UI 或其他桥接都可以实现此接口。
#[async_trait]
pub trait ApprovalBroker: Send + Sync {
    /// 解析策略生成的审批请求
    ///
    /// 返回用户的审批决定（批准/拒绝）。
    async fn request(
        &self,
        request: ApprovalRequest,
        cancel: CancelToken,
    ) -> Result<ApprovalResolution>;
}

/// 默认审批代理
///
/// 直接使用请求的默认值，无需用户交互。用于测试和不策略的场景。
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
                concurrency_safe: false,
                profiles: vec!["coding".to_string()],
                tags: vec![],
                permissions: vec![],
                side_effect: SideEffectLevel::Workspace,
                stability: StabilityLevel::Stable,
                metadata: serde_json::Value::Null,
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
