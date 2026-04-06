//! Session 状态、事件落盘与 turn 生命周期的共享基础设施。
//!
//! 这个 crate 只承载“会话真相”与“单次 turn 执行辅助”，
//! 不承担 RuntimeService 的 façade 组装，也不理解 profile / sub-agent 编排语义。

mod paths;
mod session_state;
mod support;
mod turn_runtime;

pub use paths::{display_name_from_working_dir, normalize_session_id, normalize_working_dir};
pub use session_state::{
    SessionState, SessionStateEventSink, SessionTokenBudgetState, SessionWriter,
};
pub use support::{lock_anyhow, spawn_blocking_anyhow};
pub use turn_runtime::{
    BudgetSettings, SessionTurnRunResult, append_and_broadcast,
    append_and_broadcast_from_turn_callback, complete_session_execution, execute_turn_chain,
    prepare_session_execution, run_session_turn,
};
