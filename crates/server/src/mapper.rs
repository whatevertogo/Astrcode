// 本文件只负责把内部类型投影成 HTTP DTO。
//
// 配置选择和 fallback 规则已经下沉到 runtime-config，这里只做协议层映射，
// 这样服务端入口不会悄悄长出另一套配置业务逻辑。

use astrcode_core::{
    plugin::PluginEntry, AgentEvent, CapabilityDescriptor, Phase, PluginHealth, PluginState,
    SessionEventRecord, SessionMeta,
};
use astrcode_protocol::http::{
    AgentEventEnvelope, AgentEventPayload, ConfigView, CurrentModelInfoDto, ModelOptionDto,
    OperationMetricsDto, PhaseDto, PluginHealthDto, PluginRuntimeStateDto, ProfileView,
    ReplayMetricsDto, RuntimeCapabilityDto, RuntimeMetricsDto, RuntimePluginDto, RuntimeStatusDto,
    SessionListItem, SessionMessageDto, ToolCallResultDto, ToolOutputStreamDto, PROTOCOL_VERSION,
};
use astrcode_runtime::RuntimeGovernanceSnapshot;
use astrcode_runtime::{
    is_env_var_name, list_model_options as resolve_model_options, resolve_active_selection,
    resolve_current_model as resolve_runtime_current_model, Config, OperationMetricsSnapshot,
    ReplayMetricsSnapshot, RuntimeObservabilitySnapshot, SessionMessage,
};
use axum::http::StatusCode;
use axum::response::sse::Event;

use crate::ApiError;

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

pub(crate) fn parse_event_id(raw: &str) -> Option<(u64, u32)> {
    let (storage_seq, subindex) = raw.split_once('.')?;
    Some((storage_seq.parse().ok()?, subindex.parse().ok()?))
}

pub(crate) fn format_event_id((storage_seq, subindex): (u64, u32)) -> String {
    format!("{storage_seq}.{subindex}")
}

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

fn to_tool_output_stream_dto(stream: astrcode_core::ToolOutputStream) -> ToolOutputStreamDto {
    match stream {
        astrcode_core::ToolOutputStream::Stdout => ToolOutputStreamDto::Stdout,
        astrcode_core::ToolOutputStream::Stderr => ToolOutputStreamDto::Stderr,
    }
}

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

fn to_runtime_metrics_dto(snapshot: RuntimeObservabilitySnapshot) -> RuntimeMetricsDto {
    RuntimeMetricsDto {
        session_rehydrate: to_operation_metrics_dto(snapshot.session_rehydrate),
        sse_catch_up: to_replay_metrics_dto(snapshot.sse_catch_up),
        turn_execution: to_operation_metrics_dto(snapshot.turn_execution),
    }
}

fn to_operation_metrics_dto(snapshot: OperationMetricsSnapshot) -> OperationMetricsDto {
    OperationMetricsDto {
        total: snapshot.total,
        failures: snapshot.failures,
        total_duration_ms: snapshot.total_duration_ms,
        last_duration_ms: snapshot.last_duration_ms,
        max_duration_ms: snapshot.max_duration_ms,
    }
}

fn to_replay_metrics_dto(snapshot: ReplayMetricsSnapshot) -> ReplayMetricsDto {
    ReplayMetricsDto {
        totals: to_operation_metrics_dto(snapshot.totals),
        cache_hits: snapshot.cache_hits,
        disk_fallbacks: snapshot.disk_fallbacks,
        recovered_events: snapshot.recovered_events,
    }
}

pub(crate) fn to_agent_event_dto(event: AgentEvent) -> AgentEventPayload {
    match event {
        AgentEvent::SessionStarted { session_id } => {
            AgentEventPayload::SessionStarted { session_id }
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
                duration_ms: u128::from(result.duration_ms),
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

pub(crate) fn resolve_current_model(config: &Config) -> Result<CurrentModelInfoDto, ApiError> {
    let selection = resolve_runtime_current_model(config).map_err(config_selection_error)?;

    Ok(CurrentModelInfoDto {
        profile_name: selection.profile_name,
        model: selection.model,
        provider_kind: selection.provider_kind,
    })
}

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

fn config_selection_error(error: anyhow::Error) -> ApiError {
    ApiError {
        status: StatusCode::BAD_REQUEST,
        message: error.to_string(),
    }
}

pub(crate) fn api_key_preview(api_key: Option<&str>) -> String {
    match api_key.map(str::trim) {
        None => "未配置".to_string(),
        Some("") => "未配置".to_string(),
        Some(value) if value.starts_with("env:") => {
            let env_name = value.trim_start_matches("env:").trim();
            if env_name.is_empty() {
                "未配置".to_string()
            } else {
                format!("环境变量: {}", env_name)
            }
        }
        Some(value) if value.starts_with("literal:") => {
            api_key_preview(Some(value.trim_start_matches("literal:").trim()))
        }
        Some(value) if is_env_var_name(value) && std::env::var_os(value).is_some() => {
            format!("环境变量: {}", value)
        }
        Some(value) if value.chars().count() > 4 => {
            let suffix = value
                .chars()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>();
            format!("****{}", suffix)
        }
        Some(_) => "****".to_string(),
    }
}
