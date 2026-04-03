//! AgentLoop 集成测试套件。
//!
//! 按功能拆分为多个子模块，便于维护和扩展。

mod cancellation;
mod compaction;
mod error_recovery;
mod fixtures;
mod plugin;
mod policy;
mod prompt;
mod regression;
mod test_support;
mod tool_execution;
