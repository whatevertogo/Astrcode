//! HTTP API 数据传输对象（DTO）模块
//!
//! 本模块定义了 server 与前端之间通过 HTTP/SSE 通信的所有请求/响应数据结构。
//! 所有 DTO 使用 serde 进行序列化，字段采用 camelCase 命名以匹配前端约定。
//!
//! ## 子模块划分
//!
//! - `auth`: 认证相关 DTO（bootstrap token 交换 session token）
//! - `composer`: 输入框候选列表 DTO
//! - `config`: 配置查看、保存、连接测试相关 DTO
//! - `event`: Agent 事件流 DTO，用于 SSE 实时推送和会话回放
//! - `model`: 模型信息 DTO
//! - `runtime`: 运行时状态、指标、插件健康度 DTO
//! - `session`: 会话管理 DTO（创建、列表、提示词提交）
//! - `session_event`: 会话目录事件 DTO（创建/删除/分支通知）

mod agent;
mod auth;
mod composer;
mod config;
mod event;
mod model;
mod runtime;
mod session;
mod session_event;
mod tool;

pub use agent::{
    AgentExecuteRequestDto, AgentExecuteResponseDto, AgentProfileDto, SubRunStatusDto,
    SubagentContextOverridesDto,
};
pub use auth::{AuthExchangeRequest, AuthExchangeResponse};
pub use composer::{ComposerOptionDto, ComposerOptionKindDto, ComposerOptionsResponseDto};
pub use config::{
    ConfigReloadResponse, ConfigView, ProfileView, SaveActiveSelectionRequest,
    TestConnectionRequest, TestResultDto,
};
pub use event::{
    AgentContextDto, AgentEventEnvelope, AgentEventPayload, ArtifactRefDto, CompactTriggerDto,
    ForkModeDto, InvocationKindDto, PROTOCOL_VERSION, PhaseDto, ResolvedExecutionLimitsDto,
    ResolvedSubagentContextOverridesDto, SubRunFailureCodeDto, SubRunFailureDto, SubRunHandoffDto,
    SubRunOutcomeDto, SubRunResultDto, SubRunStorageModeDto, ToolCallResultDto,
    ToolOutputStreamDto,
};
pub use model::{CurrentModelInfoDto, ModelOptionDto};
pub use runtime::{
    OperationMetricsDto, PluginHealthDto, PluginRuntimeStateDto, ReplayMetricsDto,
    RuntimeCapabilityDto, RuntimeMetricsDto, RuntimePluginDto, RuntimeReloadResponseDto,
    RuntimeStatusDto, SubRunExecutionMetricsDto,
};
pub use session::{
    CreateSessionRequest, DeleteProjectResultDto, PromptAcceptedResponse, PromptRequest,
    SessionHistoryResponseDto, SessionListItem, SessionMessageDto,
};
pub use session_event::{SessionCatalogEventEnvelope, SessionCatalogEventPayload};
pub use tool::{ToolDescriptorDto, ToolExecuteRequestDto, ToolExecuteResponseDto};
