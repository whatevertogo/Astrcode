//! # 上下文窗口管理 (Context Window Management)
//!
//! 负责 Prompt 大小估算和自动压缩的共享算法。
//!
//! ## 设计意图
//!
//! 本模块独立于 `agent_loop`，确保在线 Turn 执行和服务级手动压缩
//! 使用相同的算法和数据契约，避免逻辑分散和不一致。
//!
//! ## 子模块
//!
//! - `compaction`: 自动压缩逻辑（基于 Token 阈值或手动触发）
//! - `microcompact`: 微压缩（移除单个工具结果的冗余部分）
//! - `token_usage`: Token 估算和预算跟踪

pub(crate) mod compaction;
pub(crate) mod microcompact;
pub(crate) mod token_usage;

/// 自动压缩配置和入口函数。
pub(crate) use compaction::{auto_compact, CompactConfig};
/// 微压缩应用函数。
pub(crate) use microcompact::apply_microcompact;
/// Token 估算、预算跟踪和压缩决策相关函数。
pub(crate) use token_usage::{
    build_prompt_snapshot, effective_context_window, estimate_message_tokens,
    estimate_request_tokens, estimate_text_tokens, should_compact, TokenUsageTracker,
};
