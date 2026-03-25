use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCapabilityDto {
    pub name: String,
    pub kind: String,
    pub description: String,
    pub profiles: Vec<String>,
    pub streaming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OperationMetricsDto {
    pub total: u64,
    pub failures: u64,
    pub total_duration_ms: u64,
    pub last_duration_ms: u64,
    pub max_duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReplayMetricsDto {
    pub totals: OperationMetricsDto,
    pub cache_hits: u64,
    pub disk_fallbacks: u64,
    pub recovered_events: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeMetricsDto {
    pub session_rehydrate: OperationMetricsDto,
    pub sse_catch_up: ReplayMetricsDto,
    pub turn_execution: OperationMetricsDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PluginRuntimeStateDto {
    Discovered,
    Initialized,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PluginHealthDto {
    Unknown,
    Healthy,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimePluginDto {
    pub name: String,
    pub version: String,
    pub description: String,
    pub state: PluginRuntimeStateDto,
    pub health: PluginHealthDto,
    pub failure_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<String>,
    pub capabilities: Vec<RuntimeCapabilityDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatusDto {
    pub runtime_name: String,
    pub runtime_kind: String,
    pub loaded_session_count: usize,
    pub running_session_ids: Vec<String>,
    pub plugin_search_paths: Vec<String>,
    pub metrics: RuntimeMetricsDto,
    pub capabilities: Vec<RuntimeCapabilityDto>,
    pub plugins: Vec<RuntimePluginDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeReloadResponseDto {
    pub reloaded_at: String,
    pub status: RuntimeStatusDto,
}
