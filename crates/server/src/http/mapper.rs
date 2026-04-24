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

use astrcode_core::{SessionMeta, format_local_rfc3339};
use astrcode_host_session::{
    ComposerOption, ComposerOptionActionKind, ComposerOptionKind, SessionCatalogEvent,
};
use astrcode_protocol::http::{
    AgentExecuteResponseDto, ComposerOptionActionKindDto, ComposerOptionDto, ComposerOptionKindDto,
    ComposerOptionsResponseDto, ConfigView, CurrentModelInfoDto, ModelOptionDto, PROTOCOL_VERSION,
    PluginHealthDto, PluginRuntimeStateDto, ProfileView, ResolvedExecutionLimitsDto,
    RuntimeCapabilityDto, RuntimePluginDto, RuntimeStatusDto, SessionCatalogEventEnvelope,
    SessionCatalogEventPayload, SessionListItem, SubRunResultDto, SubRunStatusDto,
    SubRunStatusSourceDto,
};
use axum::response::sse::Event;

use crate::{
    ApiError,
    agent_api::{ServerSubRunStatusSource, ServerSubRunStatusSummary},
    config_mode_helpers,
    root_execute_service::ServerAgentExecuteSummary,
    view_projection::{
        ServerResolvedConfigSummary, ServerResolvedRuntimeStatusSummary,
        ServerRuntimeCapabilitySummary,
    },
};

fn to_runtime_capability_dto(capability: ServerRuntimeCapabilitySummary) -> RuntimeCapabilityDto {
    RuntimeCapabilityDto {
        name: capability.name,
        kind: capability.kind,
        description: capability.description,
        profiles: capability.profiles,
        streaming: capability.streaming,
    }
}

/// 将会话元数据映射为列表项 DTO。
///
/// 用于 `GET /api/sessions` 和 `POST /api/sessions` 的响应，
/// server 只负责协议包装，catalog 真相来自 `host-session`。
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

pub(crate) fn to_agent_execute_response_dto(
    summary: ServerAgentExecuteSummary,
) -> AgentExecuteResponseDto {
    AgentExecuteResponseDto {
        accepted: summary.accepted,
        message: summary.message,
        session_id: summary.session_id,
        turn_id: summary.turn_id,
        agent_id: summary.agent_id,
    }
}

pub(crate) fn to_subrun_status_dto(summary: ServerSubRunStatusSummary) -> SubRunStatusDto {
    SubRunStatusDto {
        sub_run_id: summary.sub_run_id,
        tool_call_id: summary.tool_call_id,
        source: match summary.source {
            ServerSubRunStatusSource::Live => SubRunStatusSourceDto::Live,
            ServerSubRunStatusSource::Durable => SubRunStatusSourceDto::Durable,
        },
        agent_id: summary.agent_id,
        agent_profile: summary.agent_profile,
        session_id: summary.session_id,
        child_session_id: summary.child_session_id,
        depth: summary.depth,
        parent_agent_id: summary.parent_agent_id,
        parent_sub_run_id: summary.parent_sub_run_id,
        storage_mode: summary.storage_mode,
        lifecycle: summary.lifecycle,
        last_turn_outcome: summary.last_turn_outcome,
        result: summary.result.map(to_subrun_result_dto),
        step_count: summary.step_count,
        estimated_tokens: summary.estimated_tokens,
        resolved_overrides: summary.resolved_overrides,
        resolved_limits: summary
            .resolved_limits
            .map(|_| ResolvedExecutionLimitsDto {}),
    }
}

/// 将运行时治理快照映射为运行时状态 DTO。
///
/// 包含运行时名称、类型、已加载会话数、运行中的会话 ID、
/// 插件搜索路径、运行时指标、能力描述和插件状态。
pub(crate) fn to_runtime_status_dto(
    summary: ServerResolvedRuntimeStatusSummary,
) -> RuntimeStatusDto {
    RuntimeStatusDto {
        runtime_name: summary.runtime_name,
        runtime_kind: summary.runtime_kind,
        loaded_session_count: summary.loaded_session_count,
        running_session_ids: summary.running_session_ids,
        plugin_search_paths: summary.plugin_search_paths,
        metrics: summary.metrics,
        capabilities: summary
            .capabilities
            .into_iter()
            .map(to_runtime_capability_dto)
            .collect(),
        plugins: summary
            .plugins
            .into_iter()
            .map(|plugin| RuntimePluginDto {
                name: plugin.name,
                version: plugin.version,
                description: plugin.description,
                state: to_plugin_runtime_state_dto(plugin.state),
                health: to_plugin_health_dto(plugin.health),
                failure_count: plugin.failure_count,
                failure: plugin.failure,
                warnings: plugin.warnings,
                last_checked_at: plugin.last_checked_at,
                capabilities: plugin
                    .capabilities
                    .into_iter()
                    .map(to_runtime_capability_dto)
                    .collect(),
            })
            .collect(),
    }
}

fn to_plugin_runtime_state_dto(state: astrcode_plugin_host::PluginState) -> PluginRuntimeStateDto {
    match state {
        astrcode_plugin_host::PluginState::Discovered => PluginRuntimeStateDto::Discovered,
        astrcode_plugin_host::PluginState::Initialized => PluginRuntimeStateDto::Initialized,
        astrcode_plugin_host::PluginState::Failed => PluginRuntimeStateDto::Failed,
    }
}

fn to_plugin_health_dto(health: astrcode_plugin_host::PluginHealth) -> PluginHealthDto {
    match health {
        astrcode_plugin_host::PluginHealth::Unknown => PluginHealthDto::Unknown,
        astrcode_plugin_host::PluginHealth::Healthy => PluginHealthDto::Healthy,
        astrcode_plugin_host::PluginHealth::Degraded => PluginHealthDto::Degraded,
        astrcode_plugin_host::PluginHealth::Unavailable => PluginHealthDto::Unavailable,
    }
}

/// 将会话目录事件转换为 SSE 事件。
///
/// 用于广播会话创建/删除、项目删除、会话分支等目录级变更。
/// 序列化失败时返回 `projectDeleted` 事件并携带错误信息，
/// 保证 SSE 流不会中断。
pub(crate) fn to_session_catalog_sse_event(event: SessionCatalogEvent) -> Event {
    let payload = serde_json::to_string(&SessionCatalogEventEnvelope::new(
        to_session_catalog_event_payload(event),
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

/// 构建配置视图 DTO。
///
/// server 只负责补充 `config_path` 和协议外层壳，
/// 已解析选择、profile 摘要与 API key 预览均由 application 统一提供。
pub(crate) fn build_config_view(
    summary: ServerResolvedConfigSummary,
    config_path: String,
) -> ConfigView {
    ConfigView {
        config_path,
        active_profile: summary.active_profile,
        active_model: summary.active_model,
        profiles: summary
            .profiles
            .into_iter()
            .map(|profile| ProfileView {
                name: profile.name,
                base_url: profile.base_url,
                api_key_preview: profile.api_key_preview,
                models: profile.models,
            })
            .collect(),
        warning: summary.warning,
    }
}

/// 解析当前激活的模型信息。
///
/// 从配置中提取当前使用的 profile 名称、模型名称和提供者类型，
/// 用于 `GET /api/models/current` 响应。
pub(crate) fn resolve_current_model(
    config: &astrcode_core::Config,
) -> Result<CurrentModelInfoDto, ApiError> {
    config_mode_helpers::resolve_current_model(config)
        .map_err(|error| ApiError::bad_request(error.to_string()))
}

/// 列出所有可用的模型选项。
///
/// 遍历配置中所有 profile 的模型，扁平化为列表，
/// 用于 `GET /api/models` 响应，前端据此渲染模型选择器。
pub(crate) fn list_model_options(config: &astrcode_core::Config) -> Vec<ModelOptionDto> {
    config_mode_helpers::list_model_options(config)
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

fn to_session_catalog_event_payload(event: SessionCatalogEvent) -> SessionCatalogEventPayload {
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

fn to_composer_option_dto(option: ComposerOption) -> ComposerOptionDto {
    ComposerOptionDto {
        kind: match option.kind {
            ComposerOptionKind::Command => ComposerOptionKindDto::Command,
            ComposerOptionKind::Skill => ComposerOptionKindDto::Skill,
            ComposerOptionKind::Capability => ComposerOptionKindDto::Capability,
        },
        id: option.id,
        title: option.title,
        description: option.description,
        insert_text: option.insert_text,
        action_kind: match option.action_kind {
            ComposerOptionActionKind::InsertText => ComposerOptionActionKindDto::InsertText,
            ComposerOptionActionKind::ExecuteCommand => ComposerOptionActionKindDto::ExecuteCommand,
        },
        action_value: option.action_value,
        badges: option.badges,
        keywords: option.keywords,
    }
}

fn to_subrun_result_dto(result: astrcode_core::SubRunResult) -> SubRunResultDto {
    match result {
        astrcode_core::SubRunResult::Running { handoff } => SubRunResultDto::Running { handoff },
        astrcode_core::SubRunResult::Completed { outcome, handoff } => match outcome {
            astrcode_core::CompletedSubRunOutcome::Completed => {
                SubRunResultDto::Completed { handoff }
            },
        },
        astrcode_core::SubRunResult::Failed { outcome, failure } => match outcome {
            astrcode_core::FailedSubRunOutcome::Failed => SubRunResultDto::Failed { failure },
            astrcode_core::FailedSubRunOutcome::Cancelled => SubRunResultDto::Cancelled { failure },
        },
    }
}
