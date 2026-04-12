//! 执行协调层。
//!
//! 承载从 `runtime-execution` 迁移过来的跨会话协调逻辑：
//! - 上下文快照构建与继承
//! - 执行谱系索引
//! - 执行准备（纯函数，不持有运行时状态）
//! - 子 Agent 策略解析
//!
//! kernel 只做跨会话协调，不持有会话状态（session state 在 session-runtime 中）。

mod context;
mod policy;
mod prep;

pub use context::{
    CHILD_INHERITED_COMPACT_SUMMARY_BLOCK_ID, CHILD_INHERITED_RECENT_TAIL_BLOCK_ID,
    ExecutionLineageEntry, ExecutionLineageIndex, ExecutionLineageScope,
    LINEAGE_METADATA_UNAVAILABLE_MESSAGE, ResolvedContextSnapshot, latest_compact_summary,
    recent_tail_lines, resolve_context_snapshot, single_line,
};
pub use policy::{PolicyViolation, resolve_subagent_overrides};
pub use prep::{
    AgentExecutionRequest, AgentExecutionSpec, InterruptSessionPlan, PreparedPromptSubmission,
    RootExecutionLaunch, build_background_subrun_handoff, build_child_agent_state,
    build_execution_spec, build_resumed_child_agent_state, build_root_spawn_params,
    build_subrun_failure, build_subrun_handoff, derive_child_execution_owner,
    ensure_root_execution_mode, ensure_subagent_mode, prepare_prompt_submission,
    prepare_prompt_submission_with_origin, prepare_root_execution_launch,
    resolve_interrupt_session_plan, resolve_profile_tool_names, summarize_execution_description,
    validate_root_execution_storage_mode,
};
