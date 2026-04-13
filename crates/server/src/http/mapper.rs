//! # DTO 映射层
//!
//! 本模块负责将内部领域类型（`core`/`application`/`runtime`/`storage`）投影为 HTTP 协议 DTO。
//!
//! ## 设计原则
//!
//! - **协议层映射**：配置选择和 fallback 规则已下沉到 `runtime-config`，这里只做纯映射，
//!   避免服务端入口悄悄长出另一套配置业务逻辑。
//! - **集中化**：所有 protocol 映射逻辑集中在此，`protocol` crate 保持独立， 不依赖
//!   `core`/`runtime` 的内部类型。
//! - **容错序列化**：SSE 事件序列化失败时返回结构化错误载荷而非断开连接。
//!
//! ## 映射分类
//!
//! - **会话相关**：`SessionMeta` → `SessionListItem`
//! - **运行时相关**：`GovernanceSnapshot` → `RuntimeStatusDto`
//! - **事件相关**：`AgentEvent` → `AgentEventPayload`、`SessionCatalogEvent` →
//!   `SessionCatalogEventPayload`
//! - **配置相关**：`Config` → `ConfigView`、模型选项解析
//! - **SSE 工具**：事件 ID 解析/格式化（`{storage_seq}.{subindex}` 格式）

use astrcode_application::{
    ApplicationError, ComposerOption, ComposerOptionKind, ExecutionDiagnosticsSnapshot,
    GovernanceSnapshot, OperationMetricsSnapshot, ReplayMetricsSnapshot,
    RuntimeObservabilitySnapshot, SessionCatalogEvent, SubRunExecutionMetricsSnapshot,
    is_env_var_name, list_model_options as resolve_model_options, resolve_active_selection,
    resolve_current_model as resolve_runtime_current_model,
};
use astrcode_core::{
    AgentEvent, AgentEventContext, ArtifactRef, CapabilitySpec, Config, ForkMode, InvocationMode,
    Phase, PluginHealth, PluginState, ResolvedExecutionLimitsSnapshot,
    ResolvedSubagentContextOverrides, SessionEventRecord, SessionMeta, SubRunFailure,
    SubRunFailureCode, SubRunHandoff, SubRunResult, SubRunStorageMode, SubagentContextOverrides,
    format_local_rfc3339, plugin::PluginEntry,
};
use astrcode_protocol::http::{
    AgentContextDto, AgentEventEnvelope, AgentEventPayload, AgentLifecycleDto, AgentProfileDto,
    ArtifactRefDto, ChildAgentRefDto, ChildSessionLineageKindDto, ChildSessionNotificationKindDto,
    CompactTriggerDto, ComposerOptionDto, ComposerOptionKindDto, ComposerOptionsResponseDto,
    ConfigView, CurrentModelInfoDto, ExecutionDiagnosticsDto, ForkModeDto, InvocationKindDto,
    MailboxBatchDto, MailboxDiscardedDto, MailboxQueuedDto, ModelOptionDto, OperationMetricsDto,
    PROTOCOL_VERSION, PhaseDto, PluginHealthDto, PluginRuntimeStateDto, ProfileView,
    ReplayMetricsDto, ResolvedExecutionLimitsDto, ResolvedSubagentContextOverridesDto,
    RuntimeCapabilityDto, RuntimeMetricsDto, RuntimePluginDto, RuntimeStatusDto,
    SessionCatalogEventEnvelope, SessionCatalogEventPayload, SessionListItem,
    SubRunExecutionMetricsDto, SubRunFailureCodeDto, SubRunFailureDto, SubRunHandoffDto,
    SubRunOutcomeDto, SubRunResultDto, SubRunStorageModeDto, SubagentContextOverridesDto,
    ToolCallResultDto, ToolOutputStreamDto,
};
use axum::{http::StatusCode, response::sse::Event};

use crate::ApiError;

#[derive(Debug, Clone)]
pub(crate) struct AgentProfileSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub mode: astrcode_core::AgentMode,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
}

/// 将会话元数据映射为列表项 DTO。
///
/// 用于 `GET /api/sessions` 和 `POST /api/sessions` 的响应，
/// 将时间戳转换为 RFC3339 字符串格式。
pub(crate) fn to_session_list_item(meta: SessionMeta) -> SessionListItem {
    SessionListItem {
        session_id: meta.session_id,
        working_dir: meta.working_dir,
        display_name: meta.display_name,
        title: meta.title,
        created_at: format_local_rfc3339(meta.created_at),
        updated_at: format_local_rfc3339(meta.updated_at),
        parent_session_id: meta.parent_session_id,
        parent_storage_seq: meta.parent_storage_seq,
        phase: to_phase_dto(meta.phase),
    }
}

/// 将运行时治理快照映射为运行时状态 DTO。
///
/// 包含运行时名称、类型、已加载会话数、运行中的会话 ID、
/// 插件搜索路径、运行时指标、能力描述和插件状态。
pub(crate) fn to_runtime_status_dto(snapshot: GovernanceSnapshot) -> RuntimeStatusDto {
    RuntimeStatusDto {
        runtime_name: snapshot.runtime_name,
        runtime_kind: snapshot.runtime_kind,
        loaded_session_count: snapshot.loaded_session_count,
        running_session_ids: snapshot.running_session_ids,
        plugin_search_paths: snapshot
            .plugin_search_paths
            .into_iter()
            .map(|path| path.display().to_string())
            .collect(),
        metrics: to_runtime_metrics_dto(snapshot.metrics),
        capabilities: snapshot
            .capabilities
            .into_iter()
            .map(to_runtime_capability_dto)
            .collect(),
        plugins: snapshot
            .plugins
            .into_iter()
            .map(to_runtime_plugin_dto)
            .collect(),
    }
}

pub(crate) fn to_agent_profile_dto(profile: AgentProfileSummary) -> AgentProfileDto {
    AgentProfileDto {
        id: profile.id,
        name: profile.name,
        description: profile.description,
        mode: match profile.mode {
            astrcode_core::AgentMode::Primary => "primary".to_string(),
            astrcode_core::AgentMode::SubAgent => "subAgent".to_string(),
            astrcode_core::AgentMode::All => "all".to_string(),
        },
        allowed_tools: profile.allowed_tools,
        disallowed_tools: profile.disallowed_tools,
        // TODO: 未来可能需要添加更多 agent 级执行限制摘要
    }
}

pub(crate) fn from_subagent_context_overrides_dto(
    dto: Option<SubagentContextOverridesDto>,
) -> Option<SubagentContextOverrides> {
    dto.map(|dto| SubagentContextOverrides {
        storage_mode: dto.storage_mode.map(from_subrun_storage_mode_dto),
        inherit_system_instructions: dto.inherit_system_instructions,
        inherit_project_instructions: dto.inherit_project_instructions,
        inherit_working_dir: dto.inherit_working_dir,
        inherit_policy_upper_bound: dto.inherit_policy_upper_bound,
        inherit_cancel_token: dto.inherit_cancel_token,
        include_compact_summary: dto.include_compact_summary,
        include_recent_tail: dto.include_recent_tail,
        include_recovery_refs: dto.include_recovery_refs,
        include_parent_findings: dto.include_parent_findings,
        fork_mode: dto.fork_mode.map(from_fork_mode_dto),
    })
}

fn from_fork_mode_dto(dto: ForkModeDto) -> ForkMode {
    match dto {
        ForkModeDto::FullHistory => ForkMode::FullHistory,
        ForkModeDto::LastNTurns(n) => ForkMode::LastNTurns(n),
    }
}

/// 将会话事件记录转换为 SSE 事件。
///
/// 将 `SessionEventRecord` 包装为 `AgentEventEnvelope` 并序列化为 JSON，
/// 附带协议版本号。序列化失败时返回结构化错误载荷而非 panic，
/// 确保 SSE 连接不会因单条事件序列化失败而断开。
pub(crate) fn to_sse_event(record: SessionEventRecord) -> Event {
    // Keep protocol mapping centralized so protocol stays independent from core/runtime types.
    let payload =
        serde_json::to_string(&to_agent_event_envelope(record.event)).unwrap_or_else(|error| {
            serde_json::json!({
                "protocolVersion": PROTOCOL_VERSION,
                "event": "error",
                "data": {
                    "turnId": null,
                    "code": "serialization_error",
                    "message": error.to_string()
                }
            })
            .to_string()
        });
    Event::default().id(record.event_id).data(payload)
}

/// 将 live-only 增量事件转换为 SSE 事件。
///
/// live 事件不参与 durable cursor/replay，因此故意不写 event id；
/// 断线恢复统一依赖后续 durable 真相（如 AssistantFinal / TurnDone）。
pub(crate) fn to_live_sse_event(event: AgentEvent) -> Event {
    let payload = serde_json::to_string(&to_agent_event_envelope(event)).unwrap_or_else(|error| {
        serde_json::json!({
            "protocolVersion": PROTOCOL_VERSION,
            "event": "error",
            "data": {
                "turnId": null,
                "code": "serialization_error",
                "message": error.to_string()
            }
        })
        .to_string()
    });
    Event::default().data(payload)
}

/// 将内部事件投影为 HTTP 事件信封。
///
/// 历史快照和 SSE 增量都应复用同一份 envelope 映射，避免服务端在
/// “初始化加载”和“实时事件”之间再维护两种事件载荷格式。
pub(crate) fn to_agent_event_envelope(event: AgentEvent) -> AgentEventEnvelope {
    AgentEventEnvelope {
        protocol_version: PROTOCOL_VERSION,
        event: to_agent_event_dto(event),
    }
}

/// 将会话目录事件转换为 SSE 事件。
///
/// 用于广播会话创建/删除、项目删除、会话分支等目录级变更。
/// 序列化失败时返回 `projectDeleted` 事件并携带错误信息，
/// 保证 SSE 流不会中断。
pub(crate) fn to_session_catalog_sse_event(event: SessionCatalogEvent) -> Event {
    let payload = serde_json::to_string(&SessionCatalogEventEnvelope::new(
        to_session_catalog_event_dto(event),
    ))
    .unwrap_or_else(|error| {
        serde_json::json!({
            "protocolVersion": PROTOCOL_VERSION,
            "event": "projectDeleted",
            "data": {
                "workingDir": format!("serialization-error: {error}")
            }
        })
        .to_string()
    });
    Event::default().data(payload)
}

/// 解析 SSE 事件 ID 为 `(storage_seq, subindex)` 元组。
///
/// 事件 ID 格式为 `{storage_seq}.{subindex}`，其中 `storage_seq` 是会话 writer
/// 独占分配的单调递增序号，`subindex` 用于同一存储序号下的子事件排序。
/// 解析失败返回 `None`，调用方应据此判断是否需要从磁盘回放。
pub(crate) fn parse_event_id(raw: &str) -> Option<(u64, u32)> {
    let (storage_seq, subindex) = raw.split_once('.')?;
    Some((storage_seq.parse().ok()?, subindex.parse().ok()?))
}

/// 将 `(storage_seq, subindex)` 格式化为 SSE 事件 ID 字符串。
///
/// 与 `parse_event_id` 互为逆操作，用于 SSE lag 恢复时构造游标。
pub(crate) fn format_event_id((storage_seq, subindex): (u64, u32)) -> String {
    format!("{storage_seq}.{subindex}")
}

/// 将内部 `Phase` 枚举映射为协议层 `PhaseDto`。
///
/// 阶段枚举用于前端渲染会话状态指示器（如思考中、工具调用中、流式输出等）。
pub(crate) fn to_phase_dto(phase: Phase) -> PhaseDto {
    match phase {
        Phase::Idle => PhaseDto::Idle,
        Phase::Thinking => PhaseDto::Thinking,
        Phase::CallingTool => PhaseDto::CallingTool,
        Phase::Streaming => PhaseDto::Streaming,
        Phase::Interrupted => PhaseDto::Interrupted,
        Phase::Done => PhaseDto::Done,
    }
}

/// 将工具输出流类型映射为 DTO。
///
/// 用于 `ToolCallDelta` 事件，区分 stdout 和 stderr 输出流。
fn to_tool_output_stream_dto(stream: astrcode_core::ToolOutputStream) -> ToolOutputStreamDto {
    match stream {
        astrcode_core::ToolOutputStream::Stdout => ToolOutputStreamDto::Stdout,
        astrcode_core::ToolOutputStream::Stderr => ToolOutputStreamDto::Stderr,
    }
}

fn to_compact_trigger_dto(trigger: astrcode_core::CompactTrigger) -> CompactTriggerDto {
    match trigger {
        astrcode_core::CompactTrigger::Auto => CompactTriggerDto::Auto,
        astrcode_core::CompactTrigger::Manual => CompactTriggerDto::Manual,
    }
}

fn to_agent_context_dto(agent: AgentEventContext) -> AgentContextDto {
    AgentContextDto {
        agent_id: agent.agent_id,
        parent_turn_id: agent.parent_turn_id,
        parent_sub_run_id: agent.parent_sub_run_id,
        agent_profile: agent.agent_profile,
        sub_run_id: agent.sub_run_id,
        invocation_kind: agent.invocation_kind.map(|kind| match kind {
            astrcode_core::InvocationKind::SubRun => InvocationKindDto::SubRun,
            astrcode_core::InvocationKind::RootExecution => InvocationKindDto::RootExecution,
        }),
        storage_mode: agent.storage_mode.map(to_subrun_storage_mode_dto),
        child_session_id: agent.child_session_id,
    }
}

fn to_artifact_ref_dto(artifact: ArtifactRef) -> ArtifactRefDto {
    ArtifactRefDto {
        kind: artifact.kind,
        id: artifact.id,
        label: artifact.label,
        session_id: artifact.session_id,
        storage_seq: artifact.storage_seq,
        uri: artifact.uri,
    }
}

fn from_subrun_storage_mode_dto(mode: SubRunStorageModeDto) -> SubRunStorageMode {
    match mode {
        SubRunStorageModeDto::IndependentSession => SubRunStorageMode::IndependentSession,
    }
}

fn to_subrun_storage_mode_dto(mode: SubRunStorageMode) -> SubRunStorageModeDto {
    match mode {
        SubRunStorageMode::IndependentSession => SubRunStorageModeDto::IndependentSession,
    }
}

/// 将 lifecycle + last_turn_outcome 组合映射为 SubRunOutcomeDto。
///
/// 旧 `AgentStatus` 已拆分为 `AgentLifecycleStatus`（生命周期阶段）和
/// `AgentTurnOutcome`（单轮结束原因），此函数将两者重新投影为前端兼容的 outcome 枚举。
fn to_subrun_outcome_dto(
    lifecycle: astrcode_core::AgentLifecycleStatus,
    last_turn_outcome: Option<astrcode_core::AgentTurnOutcome>,
) -> SubRunOutcomeDto {
    match last_turn_outcome {
        Some(astrcode_core::AgentTurnOutcome::Completed) => SubRunOutcomeDto::Completed,
        Some(astrcode_core::AgentTurnOutcome::Failed) => SubRunOutcomeDto::Failed,
        Some(astrcode_core::AgentTurnOutcome::Cancelled) => SubRunOutcomeDto::Aborted,
        Some(astrcode_core::AgentTurnOutcome::TokenExceeded) => SubRunOutcomeDto::TokenExceeded,
        None => match lifecycle {
            astrcode_core::AgentLifecycleStatus::Terminated => SubRunOutcomeDto::Running,
            _ => SubRunOutcomeDto::Running,
        },
    }
}

fn to_subrun_result_dto(result: SubRunResult) -> SubRunResultDto {
    SubRunResultDto {
        status: to_subrun_outcome_dto(result.lifecycle, result.last_turn_outcome),
        handoff: result.handoff.map(to_subrun_handoff_dto),
        failure: result.failure.map(to_subrun_failure_dto),
    }
}

fn to_child_agent_ref_dto(child_ref: astrcode_core::ChildAgentRef) -> ChildAgentRefDto {
    ChildAgentRefDto {
        agent_id: child_ref.agent_id,
        session_id: child_ref.session_id,
        sub_run_id: child_ref.sub_run_id,
        parent_agent_id: child_ref.parent_agent_id,
        parent_sub_run_id: child_ref.parent_sub_run_id,
        lineage_kind: to_child_lineage_kind_dto(child_ref.lineage_kind),
        status: to_agent_lifecycle_dto(child_ref.status),
        open_session_id: child_ref.open_session_id,
    }
}

fn to_agent_lifecycle_dto(status: astrcode_core::AgentLifecycleStatus) -> AgentLifecycleDto {
    match status {
        astrcode_core::AgentLifecycleStatus::Pending => AgentLifecycleDto::Pending,
        astrcode_core::AgentLifecycleStatus::Running => AgentLifecycleDto::Running,
        astrcode_core::AgentLifecycleStatus::Idle => AgentLifecycleDto::Idle,
        astrcode_core::AgentLifecycleStatus::Terminated => AgentLifecycleDto::Terminated,
    }
}

fn to_child_lineage_kind_dto(
    kind: astrcode_core::ChildSessionLineageKind,
) -> ChildSessionLineageKindDto {
    match kind {
        astrcode_core::ChildSessionLineageKind::Spawn => ChildSessionLineageKindDto::Spawn,
        astrcode_core::ChildSessionLineageKind::Fork => ChildSessionLineageKindDto::Fork,
        astrcode_core::ChildSessionLineageKind::Resume => ChildSessionLineageKindDto::Resume,
    }
}

fn to_child_notification_kind_dto(
    kind: astrcode_core::ChildSessionNotificationKind,
) -> ChildSessionNotificationKindDto {
    match kind {
        astrcode_core::ChildSessionNotificationKind::Started => {
            ChildSessionNotificationKindDto::Started
        },
        astrcode_core::ChildSessionNotificationKind::ProgressSummary => {
            ChildSessionNotificationKindDto::ProgressSummary
        },
        astrcode_core::ChildSessionNotificationKind::Delivered => {
            ChildSessionNotificationKindDto::Delivered
        },
        astrcode_core::ChildSessionNotificationKind::Waiting => {
            ChildSessionNotificationKindDto::Waiting
        },
        astrcode_core::ChildSessionNotificationKind::Resumed => {
            ChildSessionNotificationKindDto::Resumed
        },
        astrcode_core::ChildSessionNotificationKind::Closed => {
            ChildSessionNotificationKindDto::Closed
        },
        astrcode_core::ChildSessionNotificationKind::Failed => {
            ChildSessionNotificationKindDto::Failed
        },
    }
}

fn to_subrun_handoff_dto(handoff: SubRunHandoff) -> SubRunHandoffDto {
    SubRunHandoffDto {
        summary: handoff.summary,
        findings: handoff.findings,
        artifacts: handoff
            .artifacts
            .into_iter()
            .map(to_artifact_ref_dto)
            .collect(),
    }
}

fn to_subrun_failure_dto(failure: SubRunFailure) -> SubRunFailureDto {
    SubRunFailureDto {
        code: to_subrun_failure_code_dto(failure.code),
        display_message: failure.display_message,
        technical_message: failure.technical_message,
        retryable: failure.retryable,
    }
}

fn to_subrun_failure_code_dto(code: SubRunFailureCode) -> SubRunFailureCodeDto {
    match code {
        SubRunFailureCode::Transport => SubRunFailureCodeDto::Transport,
        SubRunFailureCode::ProviderHttp => SubRunFailureCodeDto::ProviderHttp,
        SubRunFailureCode::StreamParse => SubRunFailureCodeDto::StreamParse,
        SubRunFailureCode::Interrupted => SubRunFailureCodeDto::Interrupted,
        SubRunFailureCode::Internal => SubRunFailureCodeDto::Internal,
    }
}

fn to_resolved_overrides_dto(
    overrides: ResolvedSubagentContextOverrides,
) -> ResolvedSubagentContextOverridesDto {
    ResolvedSubagentContextOverridesDto {
        storage_mode: to_subrun_storage_mode_dto(overrides.storage_mode),
        inherit_system_instructions: overrides.inherit_system_instructions,
        inherit_project_instructions: overrides.inherit_project_instructions,
        inherit_working_dir: overrides.inherit_working_dir,
        inherit_policy_upper_bound: overrides.inherit_policy_upper_bound,
        inherit_cancel_token: overrides.inherit_cancel_token,
        include_compact_summary: overrides.include_compact_summary,
        include_recent_tail: overrides.include_recent_tail,
        include_recovery_refs: overrides.include_recovery_refs,
        include_parent_findings: overrides.include_parent_findings,
        fork_mode: overrides.fork_mode.map(to_fork_mode_dto),
    }
}

fn to_fork_mode_dto(fork_mode: ForkMode) -> ForkModeDto {
    match fork_mode {
        ForkMode::FullHistory => ForkModeDto::FullHistory,
        ForkMode::LastNTurns(n) => ForkModeDto::LastNTurns(n),
    }
}

fn to_resolved_limits_dto(limits: ResolvedExecutionLimitsSnapshot) -> ResolvedExecutionLimitsDto {
    ResolvedExecutionLimitsDto {
        allowed_tools: limits.allowed_tools,
        max_steps: limits.max_steps,
    }
}

/// 将能力描述符映射为 DTO。
///
/// `kind` 字段通过 serde_json 序列化后取字符串表示，
/// 反序列化失败时降级为 "unknown"，避免协议层崩溃。
fn to_runtime_capability_dto(spec: CapabilitySpec) -> RuntimeCapabilityDto {
    RuntimeCapabilityDto {
        name: spec.name.to_string(),
        kind: spec.kind.as_str().to_string(),
        description: spec.description,
        profiles: spec.profiles,
        streaming: matches!(spec.invocation_mode, InvocationMode::Streaming),
    }
}

/// 将插件条目映射为 DTO。
///
/// 包含插件清单信息（名称、版本、描述）、运行时状态、健康度、
/// 失败计数和最后检查时间，以及插件暴露的所有能力。
fn to_runtime_plugin_dto(entry: PluginEntry) -> RuntimePluginDto {
    RuntimePluginDto {
        name: entry.manifest.name,
        version: entry.manifest.version,
        description: entry.manifest.description,
        state: match entry.state {
            PluginState::Discovered => PluginRuntimeStateDto::Discovered,
            PluginState::Initialized => PluginRuntimeStateDto::Initialized,
            PluginState::Failed => PluginRuntimeStateDto::Failed,
        },
        health: match entry.health {
            PluginHealth::Unknown => PluginHealthDto::Unknown,
            PluginHealth::Healthy => PluginHealthDto::Healthy,
            PluginHealth::Degraded => PluginHealthDto::Degraded,
            PluginHealth::Unavailable => PluginHealthDto::Unavailable,
        },
        failure_count: entry.failure_count,
        failure: entry.failure,
        warnings: entry.warnings,
        last_checked_at: entry.last_checked_at,
        capabilities: entry
            .capabilities
            .into_iter()
            .map(to_runtime_capability_dto)
            .collect(),
    }
}

/// 将运行时观测指标快照映射为 DTO。
///
/// 包含三个维度的指标：会话重连（session_rehydrate）、
/// SSE 追赶（sse_catch_up）、轮次执行（turn_execution）和子执行域观测（subrun_execution）。
fn to_runtime_metrics_dto(snapshot: RuntimeObservabilitySnapshot) -> RuntimeMetricsDto {
    RuntimeMetricsDto {
        session_rehydrate: to_operation_metrics_dto(snapshot.session_rehydrate),
        sse_catch_up: to_replay_metrics_dto(snapshot.sse_catch_up),
        turn_execution: to_operation_metrics_dto(snapshot.turn_execution),
        subrun_execution: to_subrun_execution_metrics_dto(snapshot.subrun_execution),
        execution_diagnostics: to_execution_diagnostics_dto(snapshot.execution_diagnostics),
    }
}

/// 将操作指标快照映射为 DTO。
///
/// 记录总执行次数、失败次数、总耗时、最近一次耗时和最大耗时，
/// 用于前端展示运行时性能面板。
fn to_operation_metrics_dto(snapshot: OperationMetricsSnapshot) -> OperationMetricsDto {
    OperationMetricsDto {
        total: snapshot.total,
        failures: snapshot.failures,
        total_duration_ms: snapshot.total_duration_ms,
        last_duration_ms: snapshot.last_duration_ms,
        max_duration_ms: snapshot.max_duration_ms,
    }
}

/// 将回放指标快照映射为 DTO。
///
/// 在操作指标基础上增加缓存命中数、磁盘回退数和已恢复事件数，
/// 用于衡量 SSE 断线重连后的事件恢复效率。
fn to_replay_metrics_dto(snapshot: ReplayMetricsSnapshot) -> ReplayMetricsDto {
    ReplayMetricsDto {
        totals: to_operation_metrics_dto(snapshot.totals),
        cache_hits: snapshot.cache_hits,
        disk_fallbacks: snapshot.disk_fallbacks,
        recovered_events: snapshot.recovered_events,
    }
}

fn to_subrun_execution_metrics_dto(
    snapshot: SubRunExecutionMetricsSnapshot,
) -> SubRunExecutionMetricsDto {
    SubRunExecutionMetricsDto {
        total: snapshot.total,
        failures: snapshot.failures,
        completed: snapshot.completed,
        aborted: snapshot.aborted,
        token_exceeded: snapshot.token_exceeded,
        independent_session_total: snapshot.independent_session_total,
        total_duration_ms: snapshot.total_duration_ms,
        last_duration_ms: snapshot.last_duration_ms,
        total_steps: snapshot.total_steps,
        last_step_count: snapshot.last_step_count,
        total_estimated_tokens: snapshot.total_estimated_tokens,
        last_estimated_tokens: snapshot.last_estimated_tokens,
    }
}

fn to_execution_diagnostics_dto(snapshot: ExecutionDiagnosticsSnapshot) -> ExecutionDiagnosticsDto {
    ExecutionDiagnosticsDto {
        child_spawned: snapshot.child_spawned,
        child_started_persisted: snapshot.child_started_persisted,
        child_terminal_persisted: snapshot.child_terminal_persisted,
        parent_reactivation_requested: snapshot.parent_reactivation_requested,
        parent_reactivation_succeeded: snapshot.parent_reactivation_succeeded,
        parent_reactivation_failed: snapshot.parent_reactivation_failed,
        lineage_mismatch_parent_agent: snapshot.lineage_mismatch_parent_agent,
        lineage_mismatch_parent_session: snapshot.lineage_mismatch_parent_session,
        lineage_mismatch_child_session: snapshot.lineage_mismatch_child_session,
        lineage_mismatch_descriptor_missing: snapshot.lineage_mismatch_descriptor_missing,
        cache_reuse_hits: snapshot.cache_reuse_hits,
        cache_reuse_misses: snapshot.cache_reuse_misses,
        delivery_buffer_queued: snapshot.delivery_buffer_queued,
        delivery_buffer_dequeued: snapshot.delivery_buffer_dequeued,
        delivery_buffer_wake_requested: snapshot.delivery_buffer_wake_requested,
        delivery_buffer_wake_succeeded: snapshot.delivery_buffer_wake_succeeded,
        delivery_buffer_wake_failed: snapshot.delivery_buffer_wake_failed,
    }
}

/// 将智能体事件映射为协议层事件载荷。
///
/// 这是 SSE 事件流的核心映射函数，覆盖智能体生命周期中的所有事件类型：
/// 会话启动、用户消息、阶段变更、模型增量输出、思考增量、助手消息、
/// 工具调用（开始/增量/结果）、轮次完成、错误。
/// 工具调用增量输出会携带流类型（stdout/stderr）信息。
pub(crate) fn to_agent_event_dto(event: AgentEvent) -> AgentEventPayload {
    match event {
        AgentEvent::SessionStarted { session_id } => {
            AgentEventPayload::SessionStarted { session_id }
        },
        AgentEvent::UserMessage {
            turn_id,
            agent,
            content,
        } => AgentEventPayload::UserMessage {
            turn_id,
            agent: to_agent_context_dto(agent),
            content,
        },
        AgentEvent::PhaseChanged {
            turn_id,
            agent,
            phase,
        } => AgentEventPayload::PhaseChanged {
            turn_id,
            agent: to_agent_context_dto(agent),
            phase: to_phase_dto(phase),
        },
        AgentEvent::ModelDelta {
            turn_id,
            agent,
            delta,
        } => AgentEventPayload::ModelDelta {
            turn_id,
            agent: to_agent_context_dto(agent),
            delta,
        },
        AgentEvent::ThinkingDelta {
            turn_id,
            agent,
            delta,
        } => AgentEventPayload::ThinkingDelta {
            turn_id,
            agent: to_agent_context_dto(agent),
            delta,
        },
        AgentEvent::AssistantMessage {
            turn_id,
            agent,
            content,
            reasoning_content,
        } => AgentEventPayload::AssistantMessage {
            turn_id,
            agent: to_agent_context_dto(agent),
            content,
            reasoning_content,
        },
        AgentEvent::ToolCallStart {
            turn_id,
            agent,
            tool_call_id,
            tool_name,
            input,
        } => AgentEventPayload::ToolCallStart {
            turn_id,
            agent: to_agent_context_dto(agent),
            tool_call_id,
            tool_name,
            input,
        },
        AgentEvent::ToolCallDelta {
            turn_id,
            agent,
            tool_call_id,
            tool_name,
            stream,
            delta,
        } => AgentEventPayload::ToolCallDelta {
            turn_id,
            agent: to_agent_context_dto(agent),
            tool_call_id,
            tool_name,
            stream: to_tool_output_stream_dto(stream),
            delta,
        },
        AgentEvent::ToolCallResult {
            turn_id,
            agent,
            result,
        } => AgentEventPayload::ToolCallResult {
            turn_id,
            agent: to_agent_context_dto(agent),
            result: ToolCallResultDto {
                tool_call_id: result.tool_call_id,
                tool_name: result.tool_name,
                ok: result.ok,
                output: result.output,
                error: result.error,
                metadata: result.metadata,
                duration_ms: result.duration_ms,
                truncated: result.truncated,
            },
        },
        AgentEvent::CompactApplied {
            turn_id,
            agent,
            trigger,
            summary,
            preserved_recent_turns,
        } => AgentEventPayload::CompactApplied {
            turn_id,
            agent: to_agent_context_dto(agent),
            trigger: to_compact_trigger_dto(trigger),
            summary,
            preserved_recent_turns,
        },
        AgentEvent::SubRunStarted {
            turn_id,
            agent,
            tool_call_id,
            resolved_overrides,
            resolved_limits,
        } => AgentEventPayload::SubRunStarted {
            turn_id,
            agent: to_agent_context_dto(agent),
            tool_call_id,
            resolved_overrides: to_resolved_overrides_dto(resolved_overrides),
            resolved_limits: to_resolved_limits_dto(resolved_limits),
        },
        AgentEvent::SubRunFinished {
            turn_id,
            agent,
            tool_call_id,
            result,
            step_count,
            estimated_tokens,
        } => AgentEventPayload::SubRunFinished {
            turn_id,
            agent: to_agent_context_dto(agent),
            tool_call_id,
            result: to_subrun_result_dto(result),
            step_count,
            estimated_tokens,
        },
        AgentEvent::ChildSessionNotification {
            turn_id,
            agent,
            notification,
        } => AgentEventPayload::ChildSessionNotification {
            turn_id,
            agent: to_agent_context_dto(agent),
            child_ref: to_child_agent_ref_dto(notification.child_ref.clone()),
            kind: to_child_notification_kind_dto(notification.kind),
            summary: notification.summary,
            status: to_agent_lifecycle_dto(notification.status),
            source_tool_call_id: notification.source_tool_call_id,
            final_reply_excerpt: notification.final_reply_excerpt,
        },
        AgentEvent::TurnDone { turn_id, agent } => AgentEventPayload::TurnDone {
            turn_id,
            agent: to_agent_context_dto(agent),
        },
        AgentEvent::Error {
            turn_id,
            agent,
            code,
            message,
        } => AgentEventPayload::Error {
            turn_id,
            agent: to_agent_context_dto(agent),
            code,
            message,
        },
        AgentEvent::PromptMetrics {
            turn_id,
            agent,
            metrics,
        } => AgentEventPayload::PromptMetrics {
            turn_id,
            agent: to_agent_context_dto(agent),
            step_index: metrics.step_index,
            estimated_tokens: metrics.estimated_tokens,
            context_window: metrics.context_window,
            effective_window: metrics.effective_window,
            threshold_tokens: metrics.threshold_tokens,
            truncated_tool_results: metrics.truncated_tool_results,
            provider_input_tokens: metrics.provider_input_tokens,
            provider_output_tokens: metrics.provider_output_tokens,
            cache_creation_input_tokens: metrics.cache_creation_input_tokens,
            cache_read_input_tokens: metrics.cache_read_input_tokens,
            provider_cache_metrics_supported: metrics.provider_cache_metrics_supported,
            prompt_cache_reuse_hits: metrics.prompt_cache_reuse_hits,
            prompt_cache_reuse_misses: metrics.prompt_cache_reuse_misses,
        },
        AgentEvent::AgentMailboxQueued {
            turn_id,
            agent,
            payload,
        } => AgentEventPayload::AgentMailboxQueued {
            turn_id,
            agent: to_agent_context_dto(agent),
            payload: MailboxQueuedDto {
                delivery_id: payload.envelope.delivery_id,
                from_agent_id: payload.envelope.from_agent_id,
                to_agent_id: payload.envelope.to_agent_id,
                message: payload.envelope.message,
                queued_at: payload.envelope.queued_at.to_rfc3339(),
                sender_lifecycle_status: format!("{:?}", payload.envelope.sender_lifecycle_status),
                sender_last_turn_outcome: payload
                    .envelope
                    .sender_last_turn_outcome
                    .map(|outcome| format!("{outcome:?}")),
                sender_open_session_id: payload.envelope.sender_open_session_id,
                summary: None,
            },
        },
        AgentEvent::AgentMailboxBatchStarted {
            turn_id,
            agent,
            payload,
        } => AgentEventPayload::AgentMailboxBatchStarted {
            turn_id,
            agent: to_agent_context_dto(agent),
            payload: MailboxBatchDto {
                target_agent_id: payload.target_agent_id,
                turn_id: payload.turn_id,
                batch_id: payload.batch_id,
                delivery_ids: payload.delivery_ids,
            },
        },
        AgentEvent::AgentMailboxBatchAcked {
            turn_id,
            agent,
            payload,
        } => AgentEventPayload::AgentMailboxBatchAcked {
            turn_id,
            agent: to_agent_context_dto(agent),
            payload: MailboxBatchDto {
                target_agent_id: payload.target_agent_id,
                turn_id: payload.turn_id,
                batch_id: payload.batch_id,
                delivery_ids: payload.delivery_ids,
            },
        },
        AgentEvent::AgentMailboxDiscarded {
            turn_id,
            agent,
            payload,
        } => AgentEventPayload::AgentMailboxDiscarded {
            turn_id,
            agent: to_agent_context_dto(agent),
            payload: MailboxDiscardedDto {
                target_agent_id: payload.target_agent_id,
                delivery_ids: payload.delivery_ids,
            },
        },
    }
}

/// 将会话目录事件映射为协议层载荷。
///
/// 目录事件用于前端同步会话列表变更，包括会话创建/删除、
/// 项目删除（级联删除该工作目录下所有会话）、会话分支。
pub(crate) fn to_session_catalog_event_dto(
    event: SessionCatalogEvent,
) -> SessionCatalogEventPayload {
    match event {
        SessionCatalogEvent::SessionCreated { session_id } => {
            SessionCatalogEventPayload::SessionCreated { session_id }
        },
        SessionCatalogEvent::SessionDeleted { session_id } => {
            SessionCatalogEventPayload::SessionDeleted { session_id }
        },
        SessionCatalogEvent::ProjectDeleted { working_dir } => {
            SessionCatalogEventPayload::ProjectDeleted { working_dir }
        },
        SessionCatalogEvent::SessionBranched {
            session_id,
            source_session_id,
        } => SessionCatalogEventPayload::SessionBranched {
            session_id,
            source_session_id,
        },
    }
}

/// 构建配置视图 DTO。
///
/// 将内部 `Config` 转换为前端可展示的配置视图，包括：
/// - 配置文件路径
/// - 当前激活的 profile 和 model
/// - 所有 profile 列表（API key 做脱敏预览）
/// - 配置警告（如无 profile 时提示）
///
/// Profile 为空时直接返回带警告的视图，不走活跃选择解析。
pub(crate) fn build_config_view(
    config: &Config,
    config_path: String,
) -> Result<ConfigView, ApiError> {
    if config.profiles.is_empty() {
        return Ok(ConfigView {
            config_path,
            active_profile: String::new(),
            active_model: String::new(),
            profiles: Vec::new(),
            warning: Some("no profiles configured".to_string()),
        });
    }

    let profiles = config
        .profiles
        .iter()
        .map(|profile| ProfileView {
            name: profile.name.clone(),
            base_url: profile.base_url.clone(),
            api_key_preview: api_key_preview(profile.api_key.as_deref()),
            models: profile
                .models
                .iter()
                .map(|model| model.id.clone())
                .collect(),
        })
        .collect::<Vec<_>>();

    let selection = resolve_active_selection(
        &config.active_profile,
        &config.active_model,
        &config.profiles,
    )
    .map_err(config_selection_error)?;

    Ok(ConfigView {
        config_path,
        active_profile: selection.active_profile,
        active_model: selection.active_model,
        profiles,
        warning: selection.warning,
    })
}

/// 解析当前激活的模型信息。
///
/// 从配置中提取当前使用的 profile 名称、模型名称和提供者类型，
/// 用于 `GET /api/models/current` 响应。
pub(crate) fn resolve_current_model(config: &Config) -> Result<CurrentModelInfoDto, ApiError> {
    let selection = resolve_runtime_current_model(config).map_err(config_selection_error)?;

    Ok(CurrentModelInfoDto {
        profile_name: selection.profile_name,
        model: selection.model,
        provider_kind: selection.provider_kind,
    })
}

/// 列出所有可用的模型选项。
///
/// 遍历配置中所有 profile 的模型，扁平化为列表，
/// 用于 `GET /api/models` 响应，前端据此渲染模型选择器。
pub(crate) fn list_model_options(config: &Config) -> Vec<ModelOptionDto> {
    resolve_model_options(config)
        .into_iter()
        .map(|option| ModelOptionDto {
            profile_name: option.profile_name,
            model: option.model,
            provider_kind: option.provider_kind,
        })
        .collect()
}

/// 将 runtime 输入候选项映射为协议 DTO。
///
/// 保持 server 作为协议投影层，避免前端直接依赖 runtime crate 的内部枚举。
pub(crate) fn to_composer_options_response(
    items: Vec<ComposerOption>,
) -> ComposerOptionsResponseDto {
    ComposerOptionsResponseDto {
        items: items.into_iter().map(to_composer_option_dto).collect(),
    }
}

fn to_composer_option_dto(item: ComposerOption) -> ComposerOptionDto {
    ComposerOptionDto {
        kind: match item.kind {
            ComposerOptionKind::Command => ComposerOptionKindDto::Command,
            ComposerOptionKind::Skill => ComposerOptionKindDto::Skill,
            ComposerOptionKind::Capability => ComposerOptionKindDto::Capability,
        },
        id: item.id,
        title: item.title,
        description: item.description,
        insert_text: item.insert_text,
        badges: item.badges,
        keywords: item.keywords,
    }
}

fn config_selection_error(error: ApplicationError) -> ApiError {
    ApiError {
        status: StatusCode::BAD_REQUEST,
        message: error.to_string(),
    }
}

/// 生成 API key 的安全预览字符串。
///
/// 规则：
/// - `None` 或空字符串 → "未配置"
/// - `env:VAR_NAME` 前缀 → "环境变量: VAR_NAME"（不读取实际值）
/// - `literal:KEY` 前缀 → 显示 **** + 最后 4 个字符
/// - 纯大写+下划线且是有效环境变量名 → "环境变量: NAME"
/// - 长度 > 4 → 显示 "****" + 最后 4 个字符
/// - 其他 → "****"
pub(crate) fn api_key_preview(api_key: Option<&str>) -> String {
    match api_key.map(str::trim) {
        None | Some("") => "未配置".to_string(),
        Some(value) if value.starts_with("env:") => {
            let env_name = value.trim_start_matches("env:").trim();
            if env_name.is_empty() {
                "未配置".to_string()
            } else {
                format!("环境变量: {}", env_name)
            }
        },
        Some(value) if value.starts_with("literal:") => {
            let key = value.trim_start_matches("literal:").trim();
            masked_key_preview(key)
        },
        Some(value) if is_env_var_name(value) && std::env::var_os(value).is_some() => {
            format!("环境变量: {}", value)
        },
        Some(value) => masked_key_preview(value),
    }
}

fn masked_key_preview(value: &str) -> String {
    let char_starts: Vec<usize> = value.char_indices().map(|(index, _)| index).collect();

    if char_starts.len() <= 4 {
        "****".to_string()
    } else {
        // 预览语义是“最后 4 个字符”而不是“最后 4 个字节”，
        // 用字符起始位置切片可以避免多字节 UTF-8 密钥在预览时 panic。
        let suffix_start = char_starts[char_starts.len() - 4];
        format!("****{}", &value[suffix_start..])
    }
}

#[cfg(test)]
mod tests {
    use super::api_key_preview;

    #[test]
    fn api_key_preview_masks_utf8_literal_without_panicking() {
        assert_eq!(
            api_key_preview(Some("literal:令牌甲乙丙丁")),
            "****甲乙丙丁"
        );
    }

    #[test]
    fn api_key_preview_masks_utf8_plain_value_without_panicking() {
        assert_eq!(api_key_preview(Some("令牌甲乙丙丁戊")), "****乙丙丁戊");
    }
}
