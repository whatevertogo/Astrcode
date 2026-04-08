//! # 子 Agent 执行构件
//!
//! 提供子 Agent（受限 AgentLoop 执行）所需的共享逻辑。
//!
//! ## 核心类型
//! - `SubAgentPolicyEngine`: 将父策略收窄为子 Agent 可接受的能力边界，显式拒绝审批（子 Agent
//!   无独立人机交互通道）
//! - `ChildExecutionTracker`: 根据事件流追踪步数和 Token 预算，实现子 Agent 的自主运行约束
//!
//! ## 为什么放在 `runtime-agent-loop`
//!
//! 放在此 crate 而非 `runtime`，是为了让执行约束靠近执行引擎，
//! 避免 service 层同时承担“编排 + 执行细节”两类职责。

use std::collections::{HashMap, HashSet};

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
/// 因为我们需要的是”整个 child turn 的统一预算”，它既要看 prompt metrics，
/// 也要把最终输出内容一并纳入估算，事件流是当前最稳定的聚合边界。
///
/// TODO: 未来可能需要重新添加 max_steps 和 token_budget 限制功能
#[derive(Debug, Clone)]
pub struct ChildExecutionTracker {
    max_steps: Option<u32>,
    token_budget: Option<u64>,
    token_limit_hit: bool,
    step_limit_hit: bool,
    /// 记录每个 step 的最新 prompt 估算，避免重放/覆盖时重复累计。
    prompt_tokens_by_step: HashMap<u32, u64>,
    assistant_tokens: u64,
    last_summary: Option<String>,
}

impl ChildExecutionTracker {
    pub fn new(max_steps: Option<u32>, token_budget: Option<u64>) -> Self {
        Self {
            max_steps,
            token_budget,
            token_limit_hit: false,
            step_limit_hit: false,
            prompt_tokens_by_step: HashMap::new(),
            assistant_tokens: 0,
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
                self.prompt_tokens_by_step
                    .entry(*step_index)
                    .and_modify(|current| {
                        *current = (*current).max(*estimated_tokens as u64);
                    })
                    .or_insert(*estimated_tokens as u64);

                if self
                    .max_steps
                    .is_some_and(|max_steps| self.step_count() >= max_steps)
                {
                    self.step_limit_hit = true;
                    cancel.cancel();
                }
            },
            StorageEvent::AssistantFinal {
                content,
                reasoning_content,
                ..
            } => {
                let trimmed_content = content.trim();
                if !trimmed_content.is_empty() {
                    self.last_summary = Some(trimmed_content.to_string());
                }

                self.assistant_tokens = self
                    .assistant_tokens
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

        if self
            .token_budget
            .is_some_and(|token_budget| self.estimated_tokens_used() >= token_budget)
        {
            self.token_limit_hit = true;
            cancel.cancel();
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

    pub fn estimated_tokens_used(&self) -> u64 {
        self.prompt_tokens_by_step
            .values()
            .copied()
            .sum::<u64>()
            .saturating_add(self.assistant_tokens)
    }

    pub fn step_count(&self) -> u32 {
        self.prompt_tokens_by_step.len() as u32
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{AgentEventContext, StorageEvent};

    use super::ChildExecutionTracker;

    fn prompt_metrics(step_index: u32, estimated_tokens: u32) -> StorageEvent {
        StorageEvent::PromptMetrics {
            turn_id: Some("turn-1".to_string()),
            agent: AgentEventContext::default(),
            step_index,
            estimated_tokens,
            context_window: 200_000,
            effective_window: 200_000,
            threshold_tokens: 180_000,
            truncated_tool_results: 0,
            provider_input_tokens: Some(estimated_tokens),
            provider_output_tokens: None,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        }
    }

    fn assistant_final(content: &str, reasoning_content: Option<&str>) -> StorageEvent {
        StorageEvent::AssistantFinal {
            turn_id: Some("turn-1".to_string()),
            agent: AgentEventContext::default(),
            content: content.to_string(),
            reasoning_content: reasoning_content.map(ToString::to_string),
            reasoning_signature: None,
            timestamp: None,
        }
    }

    #[test]
    fn child_execution_tracker_counts_zero_based_steps() {
        let cancel = astrcode_core::CancelToken::new();
        let mut tracker = ChildExecutionTracker::new(None, None);

        tracker.observe(&prompt_metrics(0, 120), &cancel);
        tracker.observe(&prompt_metrics(1, 180), &cancel);

        assert_eq!(tracker.step_count(), 2);
        assert_eq!(tracker.estimated_tokens_used(), 300);
    }

    #[test]
    fn child_execution_tracker_cancels_when_step_limit_is_hit() {
        let cancel = astrcode_core::CancelToken::new();
        let mut tracker = ChildExecutionTracker::new(Some(2), None);

        tracker.observe(&prompt_metrics(0, 120), &cancel);
        assert!(!tracker.step_limit_hit());
        assert!(!cancel.is_cancelled());

        tracker.observe(&prompt_metrics(1, 180), &cancel);
        assert!(tracker.step_limit_hit());
        assert!(cancel.is_cancelled());
    }

    #[test]
    fn child_execution_tracker_cancels_when_token_budget_is_hit() {
        let cancel = astrcode_core::CancelToken::new();
        let mut tracker = ChildExecutionTracker::new(None, Some(20));

        tracker.observe(&prompt_metrics(0, 5), &cancel);
        assert!(!tracker.token_limit_hit());
        assert!(!cancel.is_cancelled());

        tracker.observe(
            &assistant_final(
                "final answer with enough text to exceed the remaining budget",
                Some("reasoning text"),
            ),
            &cancel,
        );
        assert!(tracker.token_limit_hit());
        assert!(cancel.is_cancelled());
    }

    #[test]
    fn child_execution_tracker_keeps_last_non_empty_summary() {
        let cancel = astrcode_core::CancelToken::new();
        let mut tracker = ChildExecutionTracker::new(None, None);

        tracker.observe(&assistant_final("first summary", None), &cancel);
        tracker.observe(&assistant_final("   ", Some("internal reasoning")), &cancel);

        assert_eq!(tracker.last_summary(), Some("first summary"));
    }
}
