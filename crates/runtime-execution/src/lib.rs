//! Agent execution 装配与上下文构建的共享逻辑。
//!
//! 这里保留纯函数与无状态装配器，让 runtime façade 不再同时承担
//! profile 裁剪、context snapshot 拼装与结果摘要等细节。

mod context;
mod prep;
mod subrun;

pub use context::{
    ResolvedContextSnapshot, latest_compact_summary, recent_tail_lines, resolve_context_snapshot,
    single_line,
};
pub use prep::{
    AgentExecutionSpec, PreparedAgentExecution, ScopedExecutionSurface, build_child_agent_state,
    build_result_artifacts, build_result_findings, derive_child_execution_owner,
    ensure_root_execution_mode, ensure_subagent_mode, prepare_scoped_agent_execution,
    resolve_profile_tool_names, resolve_subagent_overrides, summarize_child_result,
};
pub use subrun::{ParsedSubRunStatus, find_subrun_status_in_events, snapshot_from_active_handle};
