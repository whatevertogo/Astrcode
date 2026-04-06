//! # Agent 状态投影
//!
//! 从事件流（`StorageEvent` 序列）中推导出 Agent 的当前状态。
//! 该模块提供纯函数式的投影器，将 append-only 的事件日志转换为
//! 当前可操作的 Agent 状态快照。

mod agent_state;

pub use agent_state::{AgentState, AgentStateProjector, project};
