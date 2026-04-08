//! Agent 管理相关 DTO
//!
//! 定义 Agent profile 查询、执行、子执行域（sub-run）状态查询等接口的请求/响应结构。
//! 这些 DTO 用于前端展示和管理 Agent 配置、触发 Agent 执行任务以及监控 sub-run 状态。

use serde::{Deserialize, Serialize};

use crate::http::{
    ResolvedSubagentContextOverridesDto, SubRunDescriptorDto, SubRunResultDto, SubRunStorageModeDto,
};

/// 对外暴露的 Agent Profile 摘要。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfileDto {
    pub id: String,
    pub name: String,
    pub description: String,
    pub mode: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disallowed_tools: Vec<String>,
    // TODO: 未来可能需要添加 max_steps 和 token_budget 参数
}

/// `POST /api/v1/agents/{id}/execute` 请求体。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentExecuteRequestDto {
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    // TODO: 未来可能需要添加 max_steps 参数
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_overrides: Option<SubagentContextOverridesDto>,
}

/// `POST /api/v1/agents/{id}/execute` 响应体。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentExecuteResponseDto {
    pub accepted: bool,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubagentContextOverridesDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_mode: Option<SubRunStorageModeDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_system_instructions: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_project_instructions: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_working_dir: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_policy_upper_bound: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_cancel_token: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_compact_summary: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_recent_tail: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_recovery_refs: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_parent_findings: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_mode: Option<super::event::ForkModeDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SubRunStatusSourceDto {
    Live,
    Durable,
    LegacyDurable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubRunStatusDto {
    pub sub_run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptor: Option<SubRunDescriptorDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    pub source: SubRunStatusSourceDto,
    pub agent_id: String,
    pub agent_profile: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
    pub depth: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<String>,
    pub storage_mode: SubRunStorageModeDto,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<SubRunResultDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_overrides: Option<ResolvedSubagentContextOverridesDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_limits: Option<crate::http::ResolvedExecutionLimitsDto>,
}
