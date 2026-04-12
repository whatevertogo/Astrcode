//! # 上下文窗口管理
//!
//! 提供消息级别的上下文优化能力：
//! - `token_usage`: Token 估算、预算跟踪、压缩阈值计算
//! - `prune_pass`: 轻量级工具结果截断和清除（不需要 LLM）
//! - `compaction`: 上下文压缩（需要 LLM 调用生成摘要）
//! - `micro_compact`: 基于空闲时间的旧工具结果清理
//! - `file_access`: 压缩后恢复最近访问的文件上下文
//! - `request_assembler`: 最终 prompt 组装链路
//!
//! 这里不再保留旧 `compaction_runtime` / `context_pipeline` 式的根级 façade，
//! 当前真相面直接落在这些具体子模块里。

pub mod compaction;
pub mod file_access;
pub mod micro_compact;
pub mod prune_pass;
pub mod request_assembler;
pub mod token_usage;
