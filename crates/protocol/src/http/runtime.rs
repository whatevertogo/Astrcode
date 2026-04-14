//! 运行时状态与指标 DTO
//!
//! 定义运行时健康检查、指标查询、插件状态等接口的响应结构。
//! 这些数据用于前端展示系统运行状态、性能指标和插件健康度。

use serde::{Deserialize, Serialize};

/// 运行时能力的摘要信息。
///
/// 用于 `GET /api/runtime/status` 响应中列出 runtime 暴露的所有能力。
/// `profiles` 字段指示此能力在哪些 profile 下可用。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCapabilityDto {
    /// 能力名称
    pub name: String,
    /// 能力类型（如 "tool", "context_provider" 等）
    pub kind: String,
    /// 能力描述
    pub description: String,
    /// 此能力可用的 profile 列表
    pub profiles: Vec<String>,
    /// 是否支持流式输出
    pub streaming: bool,
}

/// 操作级别的指标统计。
///
/// 记录某类操作的总次数、失败次数、耗时等，用于性能监控。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OperationMetricsDto {
    /// 总操作次数
    pub total: u64,
    /// 失败次数
    pub failures: u64,
    /// 累计耗时（毫秒）
    pub total_duration_ms: u64,
    /// 最近一次操作耗时（毫秒）
    pub last_duration_ms: u64,
    /// 最大单次操作耗时（毫秒）
    pub max_duration_ms: u64,
}

/// 事件回放相关的指标。
///
/// 记录 SSE 断线重连时从磁盘/缓存恢复事件的统计信息。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReplayMetricsDto {
    /// 回放操作的总体指标
    pub totals: OperationMetricsDto,
    /// 缓存命中次数
    pub cache_hits: u64,
    /// 回退到磁盘读取的次数
    pub disk_fallbacks: u64,
    /// 成功恢复的事件数量
    pub recovered_events: u64,
}

/// 运行时整体指标。
///
/// 包含会话重连、SSE 追赶回放、turn 执行三个维度的指标。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeMetricsDto {
    /// 会话重连（rehydrate）指标
    pub session_rehydrate: OperationMetricsDto,
    /// SSE 断线重连后的回放指标
    pub sse_catch_up: ReplayMetricsDto,
    /// turn 执行指标
    pub turn_execution: OperationMetricsDto,
    /// 子执行域共享观测指标
    pub subrun_execution: SubRunExecutionMetricsDto,
    /// delivery / lineage / cache 诊断指标
    pub execution_diagnostics: ExecutionDiagnosticsDto,
    /// agent-tool 协作效果评估读模型
    pub agent_collaboration: AgentCollaborationScorecardDto,
}

/// 子执行域共享观测指标。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubRunExecutionMetricsDto {
    pub total: u64,
    pub failures: u64,
    pub completed: u64,
    pub aborted: u64,
    pub token_exceeded: u64,
    pub independent_session_total: u64,
    pub total_duration_ms: u64,
    pub last_duration_ms: u64,
    pub total_steps: u64,
    pub last_step_count: u64,
    pub total_estimated_tokens: u64,
    pub last_estimated_tokens: u64,
}

/// 结构化执行诊断指标。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionDiagnosticsDto {
    pub child_spawned: u64,
    pub child_started_persisted: u64,
    pub child_terminal_persisted: u64,
    pub parent_reactivation_requested: u64,
    pub parent_reactivation_succeeded: u64,
    pub parent_reactivation_failed: u64,
    pub lineage_mismatch_parent_agent: u64,
    pub lineage_mismatch_parent_session: u64,
    pub lineage_mismatch_child_session: u64,
    pub lineage_mismatch_descriptor_missing: u64,
    pub cache_reuse_hits: u64,
    pub cache_reuse_misses: u64,
    pub delivery_buffer_queued: u64,
    pub delivery_buffer_dequeued: u64,
    pub delivery_buffer_wake_requested: u64,
    pub delivery_buffer_wake_succeeded: u64,
    pub delivery_buffer_wake_failed: u64,
}

/// agent-tool 协作效果评估 DTO。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentCollaborationScorecardDto {
    pub total_facts: u64,
    pub spawn_accepted: u64,
    pub spawn_rejected: u64,
    pub send_reused: u64,
    pub send_queued: u64,
    pub send_rejected: u64,
    pub observe_calls: u64,
    pub observe_rejected: u64,
    pub observe_followed_by_action: u64,
    pub close_calls: u64,
    pub close_rejected: u64,
    pub delivery_delivered: u64,
    pub delivery_consumed: u64,
    pub delivery_replayed: u64,
    pub orphan_child_count: u64,
    pub child_reuse_ratio_bps: Option<u64>,
    pub observe_to_action_ratio_bps: Option<u64>,
    pub spawn_to_delivery_ratio_bps: Option<u64>,
    pub orphan_child_ratio_bps: Option<u64>,
    pub avg_delivery_latency_ms: Option<u64>,
    pub max_delivery_latency_ms: Option<u64>,
}

/// 插件运行时状态。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PluginRuntimeStateDto {
    /// 已发现但尚未初始化
    Discovered,
    /// 已初始化并可用
    Initialized,
    /// 初始化或运行期间失败
    Failed,
}

/// 插件健康度。
///
/// 用于前端展示插件的可用性状态。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PluginHealthDto {
    /// 尚未进行健康检查
    Unknown,
    /// 正常运行
    Healthy,
    /// 部分功能降级
    Degraded,
    /// 不可用
    Unavailable,
}

/// 运行时中单个插件的状态摘要。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimePluginDto {
    /// 插件名称
    pub name: String,
    /// 插件版本
    pub version: String,
    /// 插件描述
    pub description: String,
    /// 当前运行时状态
    pub state: PluginRuntimeStateDto,
    /// 健康度
    pub health: PluginHealthDto,
    /// 累计失败次数
    pub failure_count: u32,
    /// 最近一次失败的错误信息
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    /// 非致命 warning 列表。
    ///
    /// 这里显式保留 warning，是为了让前端能展示“插件已加载，但 skill 资源或
    /// allowed_tools 校验有降级”的状态，而不必把它误判为插件彻底失败。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// 最近一次健康检查的时间戳（ISO 8601）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<String>,
    /// 此插件注册的能力列表
    pub capabilities: Vec<RuntimeCapabilityDto>,
}

/// `GET /api/runtime/status` 响应体——运行时完整状态。
///
/// 包含运行时标识、活跃会话、插件状态、指标和能力列表。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatusDto {
    /// 运行时名称
    pub runtime_name: String,
    /// 运行时类型（如 "local", "remote" 等）
    pub runtime_kind: String,
    /// 当前加载的会话数量
    pub loaded_session_count: usize,
    /// 正在运行的会话 ID 列表
    pub running_session_ids: Vec<String>,
    /// 插件搜索路径列表
    pub plugin_search_paths: Vec<String>,
    /// 运行时指标
    pub metrics: RuntimeMetricsDto,
    /// 暴露的能力列表
    pub capabilities: Vec<RuntimeCapabilityDto>,
    /// 已加载的插件列表
    pub plugins: Vec<RuntimePluginDto>,
}

/// `POST /api/runtime/reload` 响应体——运行时重载后的状态。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeReloadResponseDto {
    /// 重载完成的时间戳（ISO 8601）
    pub reloaded_at: String,
    /// 重载后的运行时状态
    pub status: RuntimeStatusDto,
}
