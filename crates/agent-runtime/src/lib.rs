//! Agent 执行内核。
//!
//! 负责 turn loop、provider stream、tool dispatch、hook dispatch 和运行时上下文窗口管理。
//!
//! ## 设计说明
//!
//! - `execute_tool_calls` 当前采用**串行预检查 → 并行 I/O → 串行结果处理**的策略： 工具调度的实际
//!   I/O (`dispatch_tool`) 使用 `join_all` 并发执行， 但 Hook
//!   事件发射和结果写入保持顺序以维持消息排序确定性。
//!   如需要更细粒度的"只读并行、写串行"分桶策略，可参考旧版 `session-runtime` 的 `tool_cycle.rs`。

mod context_window;
pub mod hook_dispatch;
pub mod r#loop;
pub mod runtime;
pub mod tool_dispatch;
pub mod types;

pub use astrcode_context_window::tool_result_budget::ToolResultReplacementRecord;
pub use astrcode_runtime_contract::{
    RuntimeEventSink, RuntimeTurnEvent, TurnIdentity, TurnLoopTransition, TurnStopCause,
};
pub use hook_dispatch::{
    HookDispatchOutcome, HookDispatchRequest, HookDispatcher, HookEffect, HookEventPayload,
};
pub use r#loop::{
    StepOutcome, TurnExecutionContext, TurnExecutionResources, TurnLoop, TurnStepRunner,
};
pub use runtime::AgentRuntime;
pub use tool_dispatch::{ToolDispatchRequest, ToolDispatcher};
pub use types::{AgentRuntimeExecutionSurface, TurnInput, TurnOutput};
