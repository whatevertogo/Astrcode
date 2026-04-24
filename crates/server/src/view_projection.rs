//! server-owned HTTP projection inputs。
//!
//! 负责把共享 runtime/config 真相下沉为 server 自己的投影输入，
//! 避免路由和 mapper 直接依赖 `application` 的 summary 类型。

use astrcode_core::{CapabilitySpec, Config, InvocationMode, RuntimeObservabilitySnapshot};
use astrcode_plugin_host::{PluginEntry, PluginHealth, PluginState};

use crate::{config_mode_helpers, governance_service::ServerGovernanceSnapshot};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerConfigProfileSummary {
    pub name: String,
    pub base_url: String,
    pub api_key_preview: String,
    pub models: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerResolvedConfigSummary {
    pub active_profile: String,
    pub active_model: String,
    pub profiles: Vec<ServerConfigProfileSummary>,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerRuntimeCapabilitySummary {
    pub name: String,
    pub kind: String,
    pub description: String,
    pub profiles: Vec<String>,
    pub streaming: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerRuntimePluginSummary {
    pub name: String,
    pub version: String,
    pub description: String,
    pub state: PluginState,
    pub health: PluginHealth,
    pub failure_count: u32,
    pub failure: Option<String>,
    pub warnings: Vec<String>,
    pub last_checked_at: Option<String>,
    pub capabilities: Vec<ServerRuntimeCapabilitySummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerResolvedRuntimeStatusSummary {
    pub runtime_name: String,
    pub runtime_kind: String,
    pub loaded_session_count: usize,
    pub running_session_ids: Vec<String>,
    pub plugin_search_paths: Vec<String>,
    pub metrics: RuntimeObservabilitySnapshot,
    pub capabilities: Vec<ServerRuntimeCapabilitySummary>,
    pub plugins: Vec<ServerRuntimePluginSummary>,
}

pub(crate) fn resolve_server_config_summary(
    config: &Config,
) -> Result<ServerResolvedConfigSummary, String> {
    if config.profiles.is_empty() {
        return Ok(ServerResolvedConfigSummary {
            active_profile: String::new(),
            active_model: String::new(),
            profiles: Vec::new(),
            warning: Some("no profiles configured".to_string()),
        });
    }

    let profiles = config
        .profiles
        .iter()
        .map(|profile| ServerConfigProfileSummary {
            name: profile.name.clone(),
            base_url: profile.base_url.clone(),
            api_key_preview: api_key_preview(profile.api_key.as_deref()),
            models: profile
                .models
                .iter()
                .map(|model| model.id.clone())
                .collect(),
        })
        .collect();

    let selection = config_mode_helpers::resolve_active_selection(
        &config.active_profile,
        &config.active_model,
        &config.profiles,
    )
    .map_err(|error| error.to_string())?;

    Ok(ServerResolvedConfigSummary {
        active_profile: selection.active_profile,
        active_model: selection.active_model,
        profiles,
        warning: selection.warning,
    })
}

pub(crate) fn resolve_server_runtime_status_summary(
    snapshot: ServerGovernanceSnapshot,
) -> ServerResolvedRuntimeStatusSummary {
    ServerResolvedRuntimeStatusSummary {
        runtime_name: snapshot.runtime_name,
        runtime_kind: snapshot.runtime_kind,
        loaded_session_count: snapshot.loaded_session_count,
        running_session_ids: snapshot.running_session_ids,
        plugin_search_paths: snapshot
            .plugin_search_paths
            .into_iter()
            .map(|path| path.display().to_string())
            .collect(),
        metrics: snapshot.metrics,
        capabilities: snapshot
            .capabilities
            .into_iter()
            .map(resolve_runtime_capability_summary)
            .collect(),
        plugins: snapshot
            .plugins
            .into_iter()
            .map(resolve_runtime_plugin_summary)
            .collect(),
    }
}

fn resolve_runtime_capability_summary(spec: CapabilitySpec) -> ServerRuntimeCapabilitySummary {
    ServerRuntimeCapabilitySummary {
        name: spec.name.to_string(),
        kind: spec.kind.as_str().to_string(),
        description: spec.description,
        profiles: spec.profiles,
        streaming: matches!(spec.invocation_mode, InvocationMode::Streaming),
    }
}

fn resolve_runtime_plugin_summary(entry: PluginEntry) -> ServerRuntimePluginSummary {
    ServerRuntimePluginSummary {
        name: entry.manifest.name,
        version: entry.manifest.version,
        description: entry.manifest.description,
        state: entry.state,
        health: entry.health,
        failure_count: entry.failure_count,
        failure: entry.failure,
        warnings: entry.warnings,
        last_checked_at: entry.last_checked_at,
        capabilities: entry
            .capabilities
            .into_iter()
            .map(resolve_runtime_capability_summary)
            .collect(),
    }
}

fn api_key_preview(api_key: Option<&str>) -> String {
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
        Some(value)
            if config_mode_helpers::is_env_var_name(value) && std::env::var_os(value).is_some() =>
        {
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
        let suffix_start = char_starts[char_starts.len() - 4];
        format!("****{}", &value[suffix_start..])
    }
}
