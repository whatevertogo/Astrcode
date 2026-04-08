//! # Agent as Tool
//!
//! 提供 `spawnAgent` 工具的稳定抽象：
//! - 对 LLM 暴露统一的工具定义和参数 schema
//! - 将真实执行委托给运行时注入的 `SubAgentExecutor`
//! - 不直接依赖 `RuntimeService`，避免把 runtime 细节扩散到 Tool crate

mod executor;
mod result_mapping;
mod spawn_tool;

pub use astrcode_core::SpawnAgentParams;
pub use executor::SubAgentExecutor;
pub use spawn_tool::SpawnAgentTool;

#[cfg(test)]
mod tests;
