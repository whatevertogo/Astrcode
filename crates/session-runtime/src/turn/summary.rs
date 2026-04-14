//! Turn 级稳定汇总结构。
//!
//! 每次完整 Turn 执行结束后，由 runner 生成一份不可变汇总，
//! 供治理/诊断读取路径消费，避免上层重新扫描整条事件流。
//!
//! ## 为什么不直接用事件流
//!
//! 事件流是原始事实源，适合持久化和回放，但聚合查询代价高。
//! TurnSummary 是单次 Turn 执行的聚合视图，提供 O(1) 的指标访问。

use std::time::Duration;

use astrcode_core::{
    AgentCollaborationActionKind, AgentCollaborationFact, AgentCollaborationOutcomeKind,
};

use super::{TurnLoopTransition, TurnStopCause};

/// Turn 完成原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnFinishReason {
    /// LLM 自然结束（无工具调用，无截断）
    NaturalEnd,
    /// 用户取消
    Cancelled,
    /// 不可恢复错误
    Error,
    /// 超过 step 上限
    StepLimitExceeded,
}

impl From<TurnStopCause> for TurnFinishReason {
    fn from(value: TurnStopCause) -> Self {
        match value {
            TurnStopCause::Completed
            | TurnStopCause::BudgetStoppedContinuation
            | TurnStopCause::ContinuationLimitReached
            | TurnStopCause::MaxOutputContinuationLimitReached => Self::NaturalEnd,
            TurnStopCause::Cancelled => Self::Cancelled,
            TurnStopCause::Error => Self::Error,
            TurnStopCause::StepLimitExceeded => Self::StepLimitExceeded,
        }
    }
}

/// 单轮 turn 内的协作汇总。
#[derive(Debug, Clone, Default)]
pub struct TurnCollaborationSummary {
    pub fact_count: usize,
    pub spawn_count: usize,
    pub send_count: usize,
    pub observe_count: usize,
    pub close_count: usize,
    pub delivery_count: usize,
    pub rejected_count: usize,
    pub failed_count: usize,
    pub child_reuse_count: usize,
    pub delivery_latency_samples: usize,
    pub avg_delivery_latency_ms: Option<u64>,
    pub max_delivery_latency_ms: Option<u64>,
}

impl TurnCollaborationSummary {
    pub fn from_facts(facts: &[AgentCollaborationFact]) -> Self {
        let mut summary = Self {
            fact_count: facts.len(),
            ..Self::default()
        };
        let mut latency_total = 0u64;
        let mut max_latency = 0u64;
        for fact in facts {
            match fact.action {
                AgentCollaborationActionKind::Spawn => summary.spawn_count += 1,
                AgentCollaborationActionKind::Send => summary.send_count += 1,
                AgentCollaborationActionKind::Observe => summary.observe_count += 1,
                AgentCollaborationActionKind::Close => summary.close_count += 1,
                AgentCollaborationActionKind::Delivery => summary.delivery_count += 1,
                AgentCollaborationActionKind::ReplyToParent => {},
            }
            match fact.outcome {
                AgentCollaborationOutcomeKind::Rejected => summary.rejected_count += 1,
                AgentCollaborationOutcomeKind::Failed => summary.failed_count += 1,
                AgentCollaborationOutcomeKind::Reused => summary.child_reuse_count += 1,
                AgentCollaborationOutcomeKind::Consumed => {
                    if let Some(latency_ms) = fact.latency_ms {
                        summary.delivery_latency_samples += 1;
                        latency_total = latency_total.saturating_add(latency_ms);
                        max_latency = max_latency.max(latency_ms);
                    }
                },
                _ => {},
            }
        }
        if summary.delivery_latency_samples > 0 {
            summary.avg_delivery_latency_ms =
                Some(latency_total / summary.delivery_latency_samples as u64);
            summary.max_delivery_latency_ms = Some(max_latency);
        }
        summary
    }
}

/// 单次 Turn 执行的稳定汇总结果。
///
/// 由 `run_turn` 在 Turn 结束时生成，包含执行期间的关键指标。
/// 结构一旦生成即为不可变快照。
#[derive(Debug, Clone)]
pub struct TurnSummary {
    /// Turn 完成原因
    pub finish_reason: TurnFinishReason,
    /// 更细粒度的停止原因，供 loop/诊断层使用。
    pub stop_cause: TurnStopCause,
    /// 最后一次进入下一轮的 transition。
    pub last_transition: Option<TurnLoopTransition>,
    /// Turn 执行总耗时
    pub wall_duration: Duration,
    /// Turn 内 step 数量
    pub step_count: usize,
    /// Turn 内 budget/恢复驱动的 continuation 次数
    pub continuation_count: usize,
    /// Provider 报告的总 token 使用量（含 input + output）
    pub total_tokens_used: u64,
    /// Provider 报告的 cache read input tokens
    pub cache_read_input_tokens: u64,
    /// Provider 报告的 cache creation input tokens
    pub cache_creation_input_tokens: u64,
    /// Turn 期间发生的自动压缩次数
    pub auto_compaction_count: usize,
    /// Turn 期间发生的 reactive compact 次数
    pub reactive_compact_count: usize,
    /// Turn 期间发生的 max_tokens continuation 次数
    pub max_output_continuation_count: usize,
    /// aggregate tool-result budget 新增 replacement 的命中数
    pub tool_result_replacement_count: usize,
    /// 已有 replacement 被 durable 重放到当前 prompt 的次数
    pub tool_result_reapply_count: usize,
    /// aggregate replacement 节省的字节数
    pub tool_result_bytes_saved: u64,
    /// 进入 aggregate over-budget 处理的 tool-result message 数
    pub tool_result_over_budget_message_count: usize,
    /// 流式阶段提前启动的安全工具调用数
    pub streaming_tool_launch_count: usize,
    /// 最终与 assistant 定稿精确匹配并复用的提前执行数
    pub streaming_tool_match_count: usize,
    /// 因参数未闭合、身份未稳定或工具不安全而保守回退的次数
    pub streaming_tool_fallback_count: usize,
    /// 已启动但最终被 discard 的提前执行数
    pub streaming_tool_discard_count: usize,
    /// LLM streaming 与工具执行真实重叠的累计毫秒数
    pub streaming_tool_overlap_ms: u64,
    /// Turn 内 agent-tool 协作汇总
    pub collaboration: TurnCollaborationSummary,
}

impl TurnSummary {
    /// 计算 cache reuse 比率（0.0 ~ 1.0）。
    ///
    /// 返回 cache_read_input_tokens 占总 input tokens 的比例。
    /// 若无 token 使用记录，返回 0.0。
    pub fn cache_reuse_ratio(&self) -> f64 {
        let total_input = self
            .total_tokens_used
            .saturating_add(self.cache_read_input_tokens)
            .saturating_add(self.cache_creation_input_tokens);
        if total_input == 0 {
            return 0.0;
        }
        self.cache_read_input_tokens as f64 / total_input as f64
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{
        AgentCollaborationActionKind, AgentCollaborationFact, AgentCollaborationOutcomeKind,
        AgentCollaborationPolicyContext,
    };

    use super::TurnCollaborationSummary;

    fn fact(
        id: &str,
        action: AgentCollaborationActionKind,
        outcome: AgentCollaborationOutcomeKind,
        latency_ms: Option<u64>,
    ) -> AgentCollaborationFact {
        AgentCollaborationFact {
            fact_id: id.to_string(),
            action,
            outcome,
            parent_session_id: "session-parent".to_string(),
            turn_id: "turn-1".to_string(),
            parent_agent_id: Some("agent-root".to_string()),
            child_agent_id: Some("agent-child".to_string()),
            child_session_id: Some("session-child".to_string()),
            child_sub_run_id: Some("subrun-child".to_string()),
            delivery_id: None,
            reason_code: None,
            summary: None,
            latency_ms,
            source_tool_call_id: None,
            policy: AgentCollaborationPolicyContext {
                policy_revision: "agent-collaboration-v1".to_string(),
                max_subrun_depth: 3,
                max_spawn_per_turn: 3,
            },
        }
    }

    #[test]
    fn collaboration_summary_counts_actions_and_latency() {
        let summary = TurnCollaborationSummary::from_facts(&[
            fact(
                "spawn",
                AgentCollaborationActionKind::Spawn,
                AgentCollaborationOutcomeKind::Accepted,
                None,
            ),
            fact(
                "observe",
                AgentCollaborationActionKind::Observe,
                AgentCollaborationOutcomeKind::Rejected,
                None,
            ),
            fact(
                "send",
                AgentCollaborationActionKind::Send,
                AgentCollaborationOutcomeKind::Reused,
                None,
            ),
            fact(
                "delivery",
                AgentCollaborationActionKind::Delivery,
                AgentCollaborationOutcomeKind::Consumed,
                Some(180),
            ),
        ]);

        assert_eq!(summary.fact_count, 4);
        assert_eq!(summary.spawn_count, 1);
        assert_eq!(summary.observe_count, 1);
        assert_eq!(summary.send_count, 1);
        assert_eq!(summary.delivery_count, 1);
        assert_eq!(summary.rejected_count, 1);
        assert_eq!(summary.child_reuse_count, 1);
        assert_eq!(summary.delivery_latency_samples, 1);
        assert_eq!(summary.avg_delivery_latency_ms, Some(180));
        assert_eq!(summary.max_delivery_latency_ms, Some(180));
    }
}
