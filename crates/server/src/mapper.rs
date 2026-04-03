//! # DTO 映射层
//!
//! 本模块负责将内部领域类型（`core`/`runtime`/`storage`）投影为 HTTP 协议 DTO。
//!
//! ## 设计原则
//!
//! - **协议层映射**：配置选择和 fallback 规则已下沉到 `runtime-config`，这里只做纯映射，
//!   避免服务端入口悄悄长出另一套配置业务逻辑。
//! - **集中化**：所有 protocol 映射逻辑集中在此，`protocol` crate 保持独立，
//!   不依赖 `core`/`runtime` 的内部类型。
//! - **容错序列化**：SSE 事件序列化失败时返回结构化错误载荷而非断开连接。
//!
//! ## 映射分类
//!
//! - **会话相关**：`SessionMeta` → `SessionListItem`、`SessionMessage` → `SessionMessageDto`
//! - **运行时相关**：`RuntimeGovernanceSnapshot` → `RuntimeStatusDto`
//! - **事件相关**：`AgentEvent` → `AgentEventPayload`、`SessionCatalogEvent` → `SessionCatalogEventPayload`
//! - **配置相关**：`Config` → `ConfigView`、模型选项解析
//! - **SSE 工具**：事件 ID 解析/格式化（`{storage_seq}.{subindex}` 格式）

use astrcode_core::{
    plugin::PluginEntry, AgentEvent, AstrError, CapabilityDescriptor, Phase, PluginHealth,
    PluginState, SessionEventRecord, SessionMeta,
};
use astrcode_protocol::http::{
    AgentEventEnvelope, AgentEventPayload, ConfigView, CurrentModelInfoDto, ModelOptionDto,
    OperationMetricsDto, PhaseDto, PluginHealthDto, PluginRuntimeStateDto, ProfileView,
    ReplayMetricsDto, RuntimeCapabilityDto, RuntimeMetricsDto, RuntimePluginDto, RuntimeStatusDto,
    SessionCatalogEventEnvelope, SessionCatalogEventPayload, SessionListItem, SessionMessageDto,
    ToolCallResultDto, ToolOutputStreamDto, PROTOCOL_VERSION,
};
use astrcode_runtime::RuntimeGovernanceSnapshot;
use astrcode_runtime::{
    is_env_var_name, list_model_options as resolve_model_options, resolve_active_selection,
    resolve_current_model as resolve_runtime_current_model, Config, OperationMetricsSnapshot,
    ReplayMetricsSnapshot, RuntimeObservabilitySnapshot, SessionCatalogEvent, SessionMessage,
};
use axum::http::StatusCode;
use axum::response::sse::Event;

use crate::ApiError;

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
        created_at: meta.created_at.to_rfc3339(),
        updated_at: meta.updated_at.to_rfc3339(),
        parent_session_id: meta.parent_session_id,
        parent_storage_seq: meta.parent_storage_seq,
        phase: to_phase_dto(meta.phase),
    }
}

/// 将会话消息映射为 DTO。
///
/// 保持消息类型（User/Assistant/ToolCall）不变，仅做字段投影。
/// ToolCall 类型消息包含完整的工具调用元数据（参数、输出、错误、耗时）。
pub(crate) fn to_session_message_dto(message: SessionMessage) -> SessionMessageDto {
    match message {
        SessionMessage::User {
            turn_id,
            content,
            timestamp,
        } => SessionMessageDto::User {
            turn_id,
            content,
            timestamp,
        },
        SessionMessage::Assistant {
            turn_id,
            content,
            timestamp,
            reasoning_content,
        } => SessionMessageDto::Assistant {
            turn_id,
            content,
            timestamp,
            reasoning_content,
        },
        SessionMessage::ToolCall {
            turn_id,
            tool_call_id,
            tool_name,
            args,
            output,
            error,
            metadata,
            ok,
            duration_ms,
        } => SessionMessageDto::ToolCall {
            turn_id,
            tool_call_id,
            tool_name,
            args,
            output,
            error,
            metadata,
            ok,
            duration_ms,
        },
    }
}

/// 将运行时治理快照映射为运行时状态 DTO。
///
/// 包含运行时名称、类型、已加载会话数、运行中的会话 ID、
/// 插件搜索路径、运行时指标、能力描述和插件状态。
pub(crate) fn to_runtime_status_dto(snapshot: RuntimeGovernanceSnapshot) -> RuntimeStatusDto {
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

/// 将会话事件记录转换为 SSE 事件。
///
/// 将 `SessionEventRecord` 包装为 `AgentEventEnvelope` 并序列化为 JSON，
/// 附带协议版本号。序列化失败时返回结构化错误载荷而非 panic，
/// 确保 SSE 连接不会因单条事件序列化失败而断开。
pub(crate) fn to_sse_event(record: SessionEventRecord) -> Event {
    // Keep protocol mapping centralized so protocol stays independent from core/runtime types.
    let payload = serde_json::to_string(&AgentEventEnvelope {
        protocol_version: PROTOCOL_VERSION,
        event: to_agent_event_dto(record.event),
    })
    .unwrap_or_else(|error| {
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

/// 将能力描述符映射为 DTO。
///
/// `kind` 字段通过 serde_json 序列化后取字符串表示，
/// 反序列化失败时降级为 "unknown"，避免协议层崩溃。
fn to_runtime_capability_dto(descriptor: CapabilityDescriptor) -> RuntimeCapabilityDto {
    RuntimeCapabilityDto {
        name: descriptor.name,
        kind: serde_json::to_value(&descriptor.kind)
            .ok()
            .and_then(|value| value.as_str().map(ToString::to_string))
            .unwrap_or_else(|| "unknown".to_string()),
        description: descriptor.description,
        profiles: descriptor.profiles,
        streaming: descriptor.streaming,
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
/// SSE 追赶（sse_catch_up）、轮次执行（turn_execution）。
fn to_runtime_metrics_dto(snapshot: RuntimeObservabilitySnapshot) -> RuntimeMetricsDto {
    RuntimeMetricsDto {
        session_rehydrate: to_operation_metrics_dto(snapshot.session_rehydrate),
        sse_catch_up: to_replay_metrics_dto(snapshot.sse_catch_up),
        turn_execution: to_operation_metrics_dto(snapshot.turn_execution),
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
        }
        AgentEvent::UserMessage { turn_id, content } => {
            AgentEventPayload::UserMessage { turn_id, content }
        }
        AgentEvent::PhaseChanged { turn_id, phase } => AgentEventPayload::PhaseChanged {
            turn_id,
            phase: to_phase_dto(phase),
        },
        AgentEvent::ModelDelta { turn_id, delta } => {
            AgentEventPayload::ModelDelta { turn_id, delta }
        }
        AgentEvent::ThinkingDelta { turn_id, delta } => {
            AgentEventPayload::ThinkingDelta { turn_id, delta }
        }
        AgentEvent::AssistantMessage {
            turn_id,
            content,
            reasoning_content,
        } => AgentEventPayload::AssistantMessage {
            turn_id,
            content,
            reasoning_content,
        },
        AgentEvent::ToolCallStart {
            turn_id,
            tool_call_id,
            tool_name,
            input,
        } => AgentEventPayload::ToolCallStart {
            turn_id,
            tool_call_id,
            tool_name,
            input,
        },
        AgentEvent::ToolCallDelta {
            turn_id,
            tool_call_id,
            tool_name,
            stream,
            delta,
        } => AgentEventPayload::ToolCallDelta {
            turn_id,
            tool_call_id,
            tool_name,
            stream: to_tool_output_stream_dto(stream),
            delta,
        },
        AgentEvent::ToolCallResult { turn_id, result } => AgentEventPayload::ToolCallResult {
            turn_id,
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
        AgentEvent::TurnDone { turn_id } => AgentEventPayload::TurnDone { turn_id },
        AgentEvent::Error {
            turn_id,
            code,
            message,
        } => AgentEventPayload::Error {
            turn_id,
            code,
            message,
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
        }
        SessionCatalogEvent::SessionDeleted { session_id } => {
            SessionCatalogEventPayload::SessionDeleted { session_id }
        }
        SessionCatalogEvent::ProjectDeleted { working_dir } => {
            SessionCatalogEventPayload::ProjectDeleted { working_dir }
        }
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
            models: profile.models.clone(),
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

fn config_selection_error(error: AstrError) -> ApiError {
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
        }
        Some(value) if value.starts_with("literal:") => {
            let key = value.trim_start_matches("literal:").trim();
            if key.chars().count() > 4 {
                format!("****{}", &key[key.len() - 4..])
            } else {
                "****".to_string()
            }
        }
        Some(value) if is_env_var_name(value) && std::env::var_os(value).is_some() => {
            format!("环境变量: {}", value)
        }
        Some(value) if value.len() > 4 => format!("****{}", &value[value.len() - 4..]),
        Some(_) => "****".to_string(),
    }
}
