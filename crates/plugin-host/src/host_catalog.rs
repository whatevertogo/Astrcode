use super::PluginHostReload;
use crate::{
    RemotePluginHandshakeSummary,
    backend::{ExternalPluginRuntimeHandle, PluginBackendHealthReport},
};

/// 宿主可直接消费的 external plugin 协商目录。
///
/// 这层故意不暴露进程句柄，也不暴露完整协议对象；
/// 它只是 reload 后交给组合根的只读协商视图。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NegotiatedPluginCatalog {
    pub plugin_ids: Vec<String>,
    pub remote_plugins: Vec<NegotiatedPluginEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegotiatedPluginEntry {
    pub plugin_id: String,
    pub local_protocol_version: String,
    pub remote: Option<RemotePluginHandshakeSummary>,
}

/// external backend 健康目录。
///
/// 组合根可以通过这层看到当前 external plugin 的健康状态，
/// 不需要直接拿到底层 runtime handle。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExternalBackendHealthCatalog {
    pub reports: Vec<PluginBackendHealthReport>,
}

/// 组合根直接消费的统一运行时目录。
///
/// 这层把 active snapshot、资源目录、协商目录收成单一只读视图，
/// 让后续 server/host-session 不必分别拉三份事实源。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivePluginRuntimeCatalog {
    pub snapshot_id: String,
    pub revision: u64,
    pub plugin_ids: Vec<String>,
    pub entries: Vec<ActivePluginRuntimeEntry>,
    pub tool_names: Vec<String>,
    pub hook_ids: Vec<String>,
    pub provider_ids: Vec<String>,
    pub resource_ids: Vec<String>,
    pub command_ids: Vec<String>,
    pub theme_ids: Vec<String>,
    pub prompt_ids: Vec<String>,
    pub skill_ids: Vec<String>,
    pub negotiated_plugins: NegotiatedPluginCatalog,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivePluginRuntimeEntry {
    pub plugin_id: String,
    pub display_name: String,
    pub source_kind: crate::PluginSourceKind,
    pub enabled: bool,
    pub has_external_backend: bool,
    pub has_builtin_backend: bool,
    pub has_runtime_handle: bool,
    pub backend_health: Option<crate::backend::PluginBackendHealth>,
    pub backend_health_message: Option<String>,
    pub tool_names: Vec<String>,
    pub hook_ids: Vec<String>,
    pub provider_ids: Vec<String>,
    pub resource_ids: Vec<String>,
    pub command_ids: Vec<String>,
    pub theme_ids: Vec<String>,
    pub prompt_ids: Vec<String>,
    pub skill_ids: Vec<String>,
    pub local_protocol_version: Option<String>,
    pub remote: Option<RemotePluginHandshakeSummary>,
}

impl NegotiatedPluginCatalog {
    pub fn from_external_backends(backends: &[ExternalPluginRuntimeHandle]) -> Self {
        Self {
            plugin_ids: backends
                .iter()
                .map(|backend| backend.plugin_id.clone())
                .collect(),
            remote_plugins: backends
                .iter()
                .map(|backend| NegotiatedPluginEntry {
                    plugin_id: backend.plugin_id.clone(),
                    local_protocol_version: backend
                        .protocol_state()
                        .map(|state| state.local_initialize.protocol_version.clone())
                        .unwrap_or_default(),
                    remote: backend.remote_handshake_summary(),
                })
                .collect(),
        }
    }

    pub fn refresh_from_external_backends(&mut self, backends: &[ExternalPluginRuntimeHandle]) {
        *self = Self::from_external_backends(backends);
    }
}

impl ExternalBackendHealthCatalog {
    pub fn from_reports(reports: Vec<PluginBackendHealthReport>) -> Self {
        Self { reports }
    }

    pub fn report(&self, plugin_id: &str) -> Option<&PluginBackendHealthReport> {
        self.reports
            .iter()
            .find(|report| report.plugin_id == plugin_id)
    }
}

impl ActivePluginRuntimeCatalog {
    pub fn from_reload(reload: &PluginHostReload) -> Self {
        let entries = reload
            .descriptors
            .iter()
            .map(|descriptor| {
                let backend = reload
                    .external_backends
                    .iter()
                    .find(|backend| backend.plugin_id == descriptor.plugin_id);
                let builtin_backend = reload
                    .builtin_backends
                    .iter()
                    .find(|backend| backend.plugin_id == descriptor.plugin_id);
                let health = reload.backend_health.report(&descriptor.plugin_id);
                ActivePluginRuntimeEntry {
                    plugin_id: descriptor.plugin_id.clone(),
                    display_name: descriptor.display_name.clone(),
                    source_kind: descriptor.source_kind,
                    enabled: descriptor.enabled,
                    has_external_backend: backend.is_some(),
                    has_builtin_backend: builtin_backend.is_some(),
                    has_runtime_handle: backend.is_some() || builtin_backend.is_some(),
                    backend_health: health.map(|report| report.health.clone()),
                    backend_health_message: health.and_then(|report| report.message.clone()),
                    tool_names: descriptor
                        .tools
                        .iter()
                        .map(|tool| tool.name.to_string())
                        .collect(),
                    hook_ids: descriptor
                        .hooks
                        .iter()
                        .map(|hook| hook.hook_id.clone())
                        .collect(),
                    provider_ids: descriptor
                        .providers
                        .iter()
                        .map(|provider| provider.provider_id.clone())
                        .collect(),
                    resource_ids: descriptor
                        .resources
                        .iter()
                        .map(|resource| resource.resource_id.clone())
                        .collect(),
                    command_ids: descriptor
                        .commands
                        .iter()
                        .map(|command| command.command_id.clone())
                        .collect(),
                    theme_ids: descriptor
                        .themes
                        .iter()
                        .map(|theme| theme.theme_id.clone())
                        .collect(),
                    prompt_ids: descriptor
                        .prompts
                        .iter()
                        .map(|prompt| prompt.prompt_id.clone())
                        .collect(),
                    skill_ids: descriptor
                        .skills
                        .iter()
                        .map(|skill| skill.skill_id.clone())
                        .collect(),
                    local_protocol_version: backend.and_then(|backend| {
                        backend
                            .protocol_state()
                            .map(|state| state.local_initialize.protocol_version.clone())
                    }),
                    remote: backend.and_then(ExternalPluginRuntimeHandle::remote_handshake_summary),
                }
            })
            .collect();

        Self {
            snapshot_id: reload.snapshot.snapshot_id.clone(),
            revision: reload.snapshot.revision,
            plugin_ids: reload.snapshot.plugin_ids.clone(),
            entries,
            tool_names: reload
                .snapshot
                .tools
                .iter()
                .map(|tool| tool.name.to_string())
                .collect(),
            hook_ids: reload
                .snapshot
                .hooks
                .iter()
                .map(|hook| hook.hook_id.clone())
                .collect(),
            provider_ids: reload
                .snapshot
                .providers
                .iter()
                .map(|provider| provider.provider_id.clone())
                .collect(),
            resource_ids: reload
                .resources
                .resources
                .iter()
                .map(|resource| resource.resource_id.clone())
                .collect(),
            command_ids: reload
                .resources
                .commands
                .iter()
                .map(|command| command.command_id.clone())
                .collect(),
            theme_ids: reload
                .resources
                .themes
                .iter()
                .map(|theme| theme.theme_id.clone())
                .collect(),
            prompt_ids: reload
                .resources
                .prompts
                .iter()
                .map(|prompt| prompt.prompt_id.clone())
                .collect(),
            skill_ids: reload
                .resources
                .skills
                .iter()
                .map(|skill| skill.skill_id.clone())
                .collect(),
            negotiated_plugins: reload.negotiated_plugins.clone(),
        }
    }

    pub fn entry(&self, plugin_id: &str) -> Option<&ActivePluginRuntimeEntry> {
        self.entries
            .iter()
            .find(|entry| entry.plugin_id == plugin_id)
    }

    pub fn enabled_entries(&self) -> Vec<&ActivePluginRuntimeEntry> {
        self.entries.iter().filter(|entry| entry.enabled).collect()
    }

    pub fn tool_owner(&self, tool_name: &str) -> Option<&ActivePluginRuntimeEntry> {
        self.entries
            .iter()
            .find(|entry| entry.tool_names.iter().any(|name| name == tool_name))
    }

    pub fn hook_owner(&self, hook_id: &str) -> Option<&ActivePluginRuntimeEntry> {
        self.entries
            .iter()
            .find(|entry| entry.hook_ids.iter().any(|id| id == hook_id))
    }

    pub fn provider_owner(&self, provider_id: &str) -> Option<&ActivePluginRuntimeEntry> {
        self.entries
            .iter()
            .find(|entry| entry.provider_ids.iter().any(|id| id == provider_id))
    }

    pub fn command_owner(&self, command_id: &str) -> Option<&ActivePluginRuntimeEntry> {
        self.entries
            .iter()
            .find(|entry| entry.command_ids.iter().any(|id| id == command_id))
    }

    pub fn prompt_owner(&self, prompt_id: &str) -> Option<&ActivePluginRuntimeEntry> {
        self.entries
            .iter()
            .find(|entry| entry.prompt_ids.iter().any(|id| id == prompt_id))
    }

    pub fn skill_owner(&self, skill_id: &str) -> Option<&ActivePluginRuntimeEntry> {
        self.entries
            .iter()
            .find(|entry| entry.skill_ids.iter().any(|id| id == skill_id))
    }
}
