//! 子 Agent 执行相关的共享构件。
//!
//! 这些逻辑本质上都服务于“受限 AgentLoop 的一次执行”：
//! - `SubAgentPolicyEngine` 负责把父策略收窄为子 Agent 可接受的能力边界
//! - `ChildExecutionTracker` 负责根据事件流追踪步数 / token 预算
//!
//! 放在 `runtime-agent-loop` 而不是 `runtime`，是为了让执行约束靠近执行引擎，
//! 避免 service 层同时承担“编排 + 执行细节”两类职责。

use std::collections::HashSet;

use astrcode_core::{
    CancelToken, CapabilityCall, PolicyContext, PolicyEngine, PolicyVerdict, Result, StorageEvent,
};
use async_trait::async_trait;

use crate::estimate_text_tokens;

/// 子 Agent 的策略包装器。
///
/// 为什么这里要显式拒绝审批：
/// 当前子 Agent 没有独立的人机交互通道，如果继续把 `Ask` 往外抛，
/// 主 turn 会拿到一个它无法履约的挂起审批。
pub struct SubAgentPolicyEngine {
    parent: std::sync::Arc<dyn PolicyEngine>,
    allowed_tools: HashSet<String>,
}

impl SubAgentPolicyEngine {
    pub fn new(parent: std::sync::Arc<dyn PolicyEngine>, allowed_tools: HashSet<String>) -> Self {
        Self {
            parent,
            allowed_tools,
        }
    }
}

#[async_trait]
impl PolicyEngine for SubAgentPolicyEngine {
    async fn check_model_request(
        &self,
        request: astrcode_core::ModelRequest,
        ctx: &PolicyContext,
    ) -> Result<astrcode_core::ModelRequest> {
        self.parent.check_model_request(request, ctx).await
    }

    async fn check_capability_call(
        &self,
        call: CapabilityCall,
        ctx: &PolicyContext,
    ) -> Result<PolicyVerdict<CapabilityCall>> {
        if !self.allowed_tools.contains(call.name()) {
            return Ok(PolicyVerdict::deny(format!(
                "tool '{}' is not allowed for this sub-agent",
                call.name()
            )));
        }

        match self.parent.check_capability_call(call, ctx).await? {
            PolicyVerdict::Allow(call) => Ok(PolicyVerdict::Allow(call)),
            PolicyVerdict::Deny { reason } => Ok(PolicyVerdict::Deny { reason }),
            PolicyVerdict::Ask(pending) => Ok(PolicyVerdict::Deny {
                reason: format!(
                    "sub-agent approval requests are disabled: {}",
                    pending.request.prompt
                ),
            }),
        }
    }

    async fn decide_context_strategy(
        &self,
        input: &astrcode_core::ContextDecisionInput,
        ctx: &PolicyContext,
    ) -> Result<astrcode_core::ContextStrategy> {
        self.parent.decide_context_strategy(input, ctx).await
    }
}

/// 子 Agent 执行期间的预算追踪器。
///
/// 为什么通过事件流而不是直接绑在 LLM provider 上：
/// 因为我们需要的是“整个 child turn 的统一预算”，它既要看 prompt metrics，
/// 也要把最终输出内容一并纳入估算，事件流是当前最稳定的聚合边界。
#[derive(Debug, Clone)]
pub struct ChildExecutionTracker {
    max_steps: Option<u32>,
    token_budget: Option<u64>,
    token_limit_hit: bool,
    step_limit_hit: bool,
    estimated_tokens: u64,
    last_summary: Option<String>,
}

impl ChildExecutionTracker {
    pub fn new(max_steps: Option<u32>, token_budget: Option<u64>) -> Self {
        Self {
            max_steps,
            token_budget,
            token_limit_hit: false,
            step_limit_hit: false,
            estimated_tokens: 0,
            last_summary: None,
        }
    }

    pub fn observe(&mut self, event: &StorageEvent, cancel: &CancelToken) {
        match event {
            StorageEvent::PromptMetrics {
                step_index,
                estimated_tokens,
                ..
            } => {
                self.estimated_tokens = self
                    .estimated_tokens
                    .saturating_add(*estimated_tokens as u64);
                if let Some(max_steps) = self.max_steps {
                    if *step_index >= max_steps {
                        self.step_limit_hit = true;
                        cancel.cancel();
                    }
                }
            },
            StorageEvent::AssistantFinal {
                content,
                reasoning_content,
                ..
            } => {
                self.last_summary = Some(content.clone());
                self.estimated_tokens = self
                    .estimated_tokens
                    .saturating_add(estimate_text_tokens(content) as u64)
                    .saturating_add(
                        reasoning_content
                            .as_deref()
                            .map(estimate_text_tokens)
                            .unwrap_or_default() as u64,
                    );
            },
            _ => {},
        }

        if let Some(token_budget) = self.token_budget {
            if self.estimated_tokens >= token_budget {
                self.token_limit_hit = true;
                cancel.cancel();
            }
        }
    }

    pub fn token_limit_hit(&self) -> bool {
        self.token_limit_hit
    }

    pub fn step_limit_hit(&self) -> bool {
        self.step_limit_hit
    }

    pub fn last_summary(&self) -> Option<&str> {
        self.last_summary.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{CancelToken, StorageEvent};

    use super::ChildExecutionTracker;

    #[test]
    fn child_execution_tracker_cancels_when_step_limit_is_hit() {
        let cancel = CancelToken::new();
        let mut tracker = ChildExecutionTracker::new(Some(2), None);

        tracker.observe(
            &StorageEvent::PromptMetrics {
                turn_id: Some("turn-1".to_string()),
                agent: Default::default(),
                step_index: 2,
                estimated_tokens: 10,
                context_window: 128_000,
                effective_window: 100_000,
                threshold_tokens: 90_000,
                truncated_tool_results: 0,
            },
            &cancel,
        );

        assert!(tracker.step_limit_hit());
        assert!(cancel.is_cancelled());
    }

    #[test]
    fn child_execution_tracker_cancels_when_token_budget_is_hit() {
        let cancel = CancelToken::new();
        let mut tracker = ChildExecutionTracker::new(None, Some(8));

        tracker.observe(
            &StorageEvent::PromptMetrics {
                turn_id: Some("turn-1".to_string()),
                agent: Default::default(),
                step_index: 1,
                estimated_tokens: 8,
                context_window: 128_000,
                effective_window: 100_000,
                threshold_tokens: 90_000,
                truncated_tool_results: 0,
            },
            &cancel,
        );

        assert!(tracker.token_limit_hit());
        assert!(cancel.is_cancelled());
    }
}
