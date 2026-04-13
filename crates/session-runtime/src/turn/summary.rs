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

/// Turn 完成原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnFinishReason {
    /// LLM 自然结束（无工具调用，无截断）
    NaturalEnd,
    /// 预算耗尽
    BudgetExhausted,
    /// 收益递减（增量过小）
    DiminishingReturns,
    /// 用户取消
    Cancelled,
    /// 不可恢复错误
    Error,
    /// 超过 step 上限
    StepLimitExceeded,
}

/// 单次 Turn 执行的稳定汇总结果。
///
/// 由 `run_turn` 在 Turn 结束时生成，包含执行期间的关键指标。
/// 结构一旦生成即为不可变快照。
#[derive(Debug, Clone)]
pub struct TurnSummary {
    /// Turn 完成原因
    pub finish_reason: TurnFinishReason,
    /// Turn 执行总耗时
    pub wall_duration: Duration,
    /// Turn 内 step 数量
    pub step_count: usize,
    /// 自动续写次数
    pub continuation_count: u8,
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
