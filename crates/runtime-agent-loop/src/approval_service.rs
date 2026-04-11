//! # Approval Broker（审批代理）
//!
//! ## 职责
//!
//! 负责处理需要用户确认的能力调用，实现交互式审批流程。
//! 当策略引擎判定某个工具调用需要审批时，AgentLoop 会通过此接口阻塞等待用户决定。
//!
//! ## 在 Turn 流程中的作用
//!
//! - **调用时机**：工具执行前，`tool_cycle` 在策略返回 `RequiresApproval` 后调用
//! - **输入**：`ApprovalRequest`（工具名、参数、策略给出的默认决策）
//! - **输出**：`ApprovalResolution`（Allow/Deny/AllowModified）
//! - **阻塞行为**：调用方会等待直到用户响应或 `CancelToken` 触发
//!
//! ## 依赖和协作
//!
//! - **使用** `astrcode_core::{ApprovalRequest, ApprovalResolution, CancelToken}`
//! - **被调用方**：`tool_cycle` 中的 `ask_approval()` 辅助函数
//! - **传输无关**：trait 设计刻意与传输层解耦，CLI、Web UI、Tauri 均可实现此接口
//!
//! ## 默认实现
//!
//! `DefaultApprovalBroker` 直接返回请求的默认决策（无需用户交互），用于测试和无审批场景。

use astrcode_core::{ApprovalRequest, ApprovalResolution, CancelToken, Result};
use async_trait::async_trait;

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
    use astrcode_core::{ApprovalDefault, ApprovalRequest, CancelToken};
    use astrcode_protocol::capability::{
        CapabilityDescriptor, CapabilityKind, SideEffectLevel, StabilityLevel,
    };
    use serde_json::json;

    use super::{ApprovalBroker, DefaultApprovalBroker};

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
                compact_clearable: false,
                profiles: vec!["coding".to_string()],
                tags: vec![],
                permissions: vec![],
                side_effect: SideEffectLevel::Workspace,
                stability: StabilityLevel::Stable,
                metadata: serde_json::Value::Null,
                max_result_inline_size: None,
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
