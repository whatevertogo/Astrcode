//! # DTO 映射层
//!
//! 本模块负责将内部领域类型（`core`/`application`/`runtime`/`storage`）投影为 HTTP 协议 DTO。
//!
//! ## 设计原则
//!
//! - **协议层映射**：配置选择和 fallback 规则已下沉到 `runtime-config`，这里只做纯映射，
//!   避免服务端入口悄悄长出另一套配置业务逻辑。
//! - **集中化**：所有 protocol 映射逻辑集中在此；纯镜像类型已在 `protocol` 中直接复用 `core`
//!   定义，这里只保留真正的协议投影。
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
    AgentCollaborationScorecardSnapshot, AgentMode, ApplicationError, CapabilitySpec,
    ComposerOption, ComposerOptionKind, Config, ExecutionDiagnosticsSnapshot, GovernanceSnapshot,
    InvocationMode, OperationMetricsSnapshot, PluginEntry, PluginHealth, PluginState,
    ReplayMetricsSnapshot, RuntimeObservabilitySnapshot, SessionCatalogEvent, SessionMeta,
    SubRunExecutionMetricsSnapshot, SubagentContextOverrides, format_local_rfc3339,
    is_env_var_name, list_model_options as resolve_model_options, resolve_active_selection,
    resolve_current_model as resolve_runtime_current_model,
};
use astrcode_protocol::http::{
    AgentCollaborationScorecardDto, AgentProfileDto, ComposerOptionActionKindDto,
    ComposerOptionDto, ComposerOptionKindDto, ComposerOptionsResponseDto, ConfigView,
    CurrentModelInfoDto, ExecutionDiagnosticsDto, ModelOptionDto, OperationMetricsDto,
    PROTOCOL_VERSION, PluginHealthDto, PluginRuntimeStateDto, ProfileView, ReplayMetricsDto,
    RuntimeCapabilityDto, RuntimeMetricsDto, RuntimePluginDto, RuntimeStatusDto,
    SessionCatalogEventEnvelope, SessionCatalogEventPayload, SessionListItem,
    SubRunExecutionMetricsDto, SubagentContextOverridesDto,
};
use axum::{http::StatusCode, response::sse::Event};

use crate::ApiError;

#[derive(Debug, Clone)]
pub(crate) struct AgentProfileSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub mode: AgentMode,
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
        phase: meta.phase,
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
            AgentMode::Primary => "primary".to_string(),
            AgentMode::SubAgent => "subAgent".to_string(),
            AgentMode::All => "all".to_string(),
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
        storage_mode: dto.storage_mode,
        inherit_system_instructions: dto.inherit_system_instructions,
        inherit_project_instructions: dto.inherit_project_instructions,
        inherit_working_dir: dto.inherit_working_dir,
        inherit_policy_upper_bound: dto.inherit_policy_upper_bound,
        inherit_cancel_token: dto.inherit_cancel_token,
        include_compact_summary: dto.include_compact_summary,
        include_recent_tail: dto.include_recent_tail,
        include_recovery_refs: dto.include_recovery_refs,
        include_parent_findings: dto.include_parent_findings,
        fork_mode: dto.fork_mode,
    })
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
pub(crate) fn to_runtime_metrics_dto(snapshot: RuntimeObservabilitySnapshot) -> RuntimeMetricsDto {
    RuntimeMetricsDto {
        session_rehydrate: to_operation_metrics_dto(snapshot.session_rehydrate),
        sse_catch_up: to_replay_metrics_dto(snapshot.sse_catch_up),
        turn_execution: to_operation_metrics_dto(snapshot.turn_execution),
        subrun_execution: to_subrun_execution_metrics_dto(snapshot.subrun_execution),
        execution_diagnostics: to_execution_diagnostics_dto(snapshot.execution_diagnostics),
        agent_collaboration: to_agent_collaboration_scorecard_dto(snapshot.agent_collaboration),
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
        cancelled: snapshot.cancelled,
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

fn to_agent_collaboration_scorecard_dto(
    snapshot: AgentCollaborationScorecardSnapshot,
) -> AgentCollaborationScorecardDto {
    AgentCollaborationScorecardDto {
        total_facts: snapshot.total_facts,
        spawn_accepted: snapshot.spawn_accepted,
        spawn_rejected: snapshot.spawn_rejected,
        send_reused: snapshot.send_reused,
        send_queued: snapshot.send_queued,
        send_rejected: snapshot.send_rejected,
        observe_calls: snapshot.observe_calls,
        observe_rejected: snapshot.observe_rejected,
        observe_followed_by_action: snapshot.observe_followed_by_action,
        close_calls: snapshot.close_calls,
        close_rejected: snapshot.close_rejected,
        delivery_delivered: snapshot.delivery_delivered,
        delivery_consumed: snapshot.delivery_consumed,
        delivery_replayed: snapshot.delivery_replayed,
        orphan_child_count: snapshot.orphan_child_count,
        child_reuse_ratio_bps: snapshot.child_reuse_ratio_bps,
        observe_to_action_ratio_bps: snapshot.observe_to_action_ratio_bps,
        spawn_to_delivery_ratio_bps: snapshot.spawn_to_delivery_ratio_bps,
        orphan_child_ratio_bps: snapshot.orphan_child_ratio_bps,
        avg_delivery_latency_ms: snapshot.avg_delivery_latency_ms,
        max_delivery_latency_ms: snapshot.max_delivery_latency_ms,
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
        action_kind: match item.action_kind {
            astrcode_application::ComposerOptionActionKind::InsertText => {
                ComposerOptionActionKindDto::InsertText
            },
            astrcode_application::ComposerOptionActionKind::ExecuteCommand => {
                ComposerOptionActionKindDto::ExecuteCommand
            },
        },
        action_value: item.action_value,
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
