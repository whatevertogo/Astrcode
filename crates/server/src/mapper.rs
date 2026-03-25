use astrcode_core::{
    plugin::PluginEntry, AgentEvent, CapabilityDescriptor, Phase, PluginHealth, PluginState,
    SessionEventRecord, SessionMeta,
};
use astrcode_protocol::http::{
    AgentEventEnvelope, AgentEventPayload, ConfigView, CurrentModelInfoDto, ModelOptionDto,
    OperationMetricsDto, PhaseDto, PluginHealthDto, PluginRuntimeStateDto, ProfileView,
    ReplayMetricsDto, RuntimeCapabilityDto, RuntimeMetricsDto, RuntimePluginDto, RuntimeStatusDto,
    SessionListItem, SessionMessageDto, ToolCallResultDto, PROTOCOL_VERSION,
};
use astrcode_runtime::{
    Config, OperationMetricsSnapshot, Profile, ReplayMetricsSnapshot, RuntimeObservabilitySnapshot,
    SessionMessage,
};
use axum::http::StatusCode;
use axum::response::sse::Event;

use crate::capabilities::RuntimeGovernanceSnapshot;
use crate::ApiError;

pub(crate) fn to_session_list_item(meta: SessionMeta) -> SessionListItem {
    SessionListItem {
        session_id: meta.session_id,
        working_dir: meta.working_dir,
        display_name: meta.display_name,
        title: meta.title,
        created_at: meta.created_at.to_rfc3339(),
        updated_at: meta.updated_at.to_rfc3339(),
        phase: to_phase_dto(meta.phase),
    }
}

pub(crate) fn to_session_message_dto(message: SessionMessage) -> SessionMessageDto {
    match message {
        SessionMessage::User { content, timestamp } => {
            SessionMessageDto::User { content, timestamp }
        }
        SessionMessage::Assistant {
            content,
            timestamp,
            reasoning_content,
        } => SessionMessageDto::Assistant {
            content,
            timestamp,
            reasoning_content,
        },
        SessionMessage::ToolCall {
            tool_call_id,
            tool_name,
            args,
            output,
            ok,
            duration_ms,
        } => SessionMessageDto::ToolCall {
            tool_call_id,
            tool_name,
            args,
            output,
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

    let (active_profile, active_model, warning) = resolve_active_selection(
        &config.active_profile,
        &config.active_model,
        &config.profiles,
    )?;

    Ok(ConfigView {
        config_path,
        active_profile,
        active_model,
        profiles,
        warning,
    })
}

pub(crate) fn resolve_current_model(config: &Config) -> Result<CurrentModelInfoDto, ApiError> {
    let profile = config
        .profiles
        .iter()
        .find(|profile| profile.name == config.active_profile)
        .or_else(|| config.profiles.first())
        .ok_or_else(|| ApiError {
            status: StatusCode::BAD_REQUEST,
            message: "no profiles configured".to_string(),
        })?;

    let model = if profile
        .models
        .iter()
        .any(|item| item == &config.active_model)
    {
        config.active_model.clone()
    } else {
        profile.models.first().cloned().ok_or_else(|| ApiError {
            status: StatusCode::BAD_REQUEST,
            message: format!("profile '{}' has no models", profile.name),
        })?
    };

    Ok(CurrentModelInfoDto {
        profile_name: profile.name.clone(),
        model,
        provider_kind: profile.provider_kind.clone(),
    })
}

pub(crate) fn list_model_options(config: &Config) -> Vec<ModelOptionDto> {
    config
        .profiles
        .iter()
        .flat_map(|profile| {
            profile.models.iter().map(|model| ModelOptionDto {
                profile_name: profile.name.clone(),
                model: model.clone(),
                provider_kind: profile.provider_kind.clone(),
            })
        })
        .collect()
}

fn resolve_active_selection(
    active_profile: &str,
    active_model: &str,
    profiles: &[Profile],
) -> Result<(String, String, Option<String>), ApiError> {
    let fallback_profile = profiles.first().ok_or_else(|| ApiError {
        status: StatusCode::BAD_REQUEST,
        message: "no profiles configured".to_string(),
    })?;

    let selected_profile = profiles
        .iter()
        .find(|profile| profile.name == active_profile)
        .unwrap_or(fallback_profile);

    if selected_profile.models.is_empty() {
        return Err(ApiError {
            status: StatusCode::BAD_REQUEST,
            message: format!("profile '{}' has no models", selected_profile.name),
        });
    }

    if selected_profile.name != active_profile {
        return Ok((
            selected_profile.name.clone(),
            selected_profile.models[0].clone(),
            Some(format!(
                "配置中的 Profile 不存在，已自动选择 {}",
                selected_profile.name
            )),
        ));
    }

    if let Some(model) = selected_profile
        .models
        .iter()
        .find(|model| *model == active_model)
    {
        return Ok((selected_profile.name.clone(), model.clone(), None));
    }

    Ok((
        selected_profile.name.clone(),
        selected_profile.models[0].clone(),
        Some(format!(
            "配置中的 {} 在当前 Profile 下不存在，已自动选择 {}",
            active_model, selected_profile.models[0]
        )),
    ))
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

fn is_env_var_name(value: &str) -> bool {
    value
        .chars()
        .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        && value.contains('_')
}
