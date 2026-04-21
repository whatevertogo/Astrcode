use serde::{Deserialize, Serialize};

use super::{
    AgentMode, ForkMode, normalize_non_empty_unique_string_list, require_non_empty_trimmed,
    require_not_whitespace_only,
};
use crate::error::{AstrError, Result};

/// `spawn` 的稳定调用参数。
///
/// 该 DTO 下沉到 core，是为了让工具层和执行装配层共享同一份参数语义，
/// 避免 `runtime-execution` 只为了复用字段定义而反向依赖 `runtime-agent-tool`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpawnCapabilityGrant {
    /// 本次 child 允许使用的 tool capability names。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
}

impl SpawnCapabilityGrant {
    pub fn validate(&self) -> Result<()> {
        let normalized = normalize_non_empty_unique_string_list(
            &self.allowed_tools,
            "capabilityGrant.allowedTools",
        )?;
        if normalized.is_empty() {
            return Err(AstrError::Validation(
                "capabilityGrant.allowedTools 不能为空".to_string(),
            ));
        }
        Ok(())
    }

    pub fn normalized_allowed_tools(&self) -> Result<Vec<String>> {
        normalize_non_empty_unique_string_list(&self.allowed_tools, "capabilityGrant.allowedTools")
    }
}

/// `spawn` 的稳定调用参数。
///
/// 该 DTO 下沉到 core，是为了让工具层和执行装配层共享同一份参数语义，
/// 避免 `runtime-execution` 只为了复用字段定义而反向依赖 `runtime-agent-tool`。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpawnAgentParams {
    /// Agent profile 标识。留空默认 "explore"。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,

    /// 短摘要，给 UI / 标题 / 日志展示用。不参与任务语义。
    pub description: String,

    /// 任务正文。子 Agent 收到的指令主体。必填。
    pub prompt: String,

    /// 可选补充材料。不保证完整历史，只是附加信息。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,

    /// 本次任务级 capability grant。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_grant: Option<SpawnCapabilityGrant>,
}

impl SpawnAgentParams {
    /// 校验参数合法性。
    pub fn validate(&self) -> Result<()> {
        // prompt 是子 Agent 收到的指令主体，不能为空；
        // 否则 runtime 只能启动一个没有任务语义的空会话。
        require_non_empty_trimmed("prompt", &self.prompt)?;
        // description 只承担可观测性职责；
        // 允许空串兼容模型输出，但纯空白会污染标题与日志。
        require_not_whitespace_only("description", &self.description)?;
        if let Some(grant) = &self.capability_grant {
            grant.validate()?;
        }
        Ok(())
    }
}

/// 调用侧可传入的子会话上下文 override。
///
/// 使用 `Option` 字段而不是硬编码完整配置，原因是调用方通常只覆写极少数字段；
/// 其余维度应继续沿用 runtime 的默认强隔离策略。
///
/// ## 当前约束
///
/// 以下字段有运行时限制，不是所有值都支持：
///
/// - `inherit_cancel_token`: 不支持设为 `false`。原因是取消必须级联传播， 否则父 Agent 取消后子
///   Agent 会成为孤儿进程继续运行，造成资源泄漏。 TODO: 未来可考虑实现独立的子 Agent
///   超时机制，允许有限度的取消隔离。
///
/// - `include_recovery_refs`: 不支持设为 `true`。恢复引用涉及复杂的跨会话状态依赖， 当前子 Agent
///   执行模型不保证这些引用在子会话中仍然有效。 TODO: 需要先设计跨会话引用的稳定协议后才能开放。
///
/// - `include_parent_findings`: 不支持设为 `true`。父 Agent 的 findings 是非结构化的，
///   直接注入可能导致上下文污染或意外行为。 TODO: 需要先定义 findings 的结构化格式和过滤机制。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubagentContextOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_mode: Option<super::SubRunStorageMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_system_instructions: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_project_instructions: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_working_dir: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_policy_upper_bound: Option<bool>,
    /// 取消令牌继承。**不支持设为 false**，见结构体文档说明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherit_cancel_token: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_compact_summary: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_recent_tail: Option<bool>,
    /// 恢复引用包含。**不支持设为 true**，见结构体文档说明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_recovery_refs: Option<bool>,
    /// 父 Agent findings 包含。**不支持设为 true**，见结构体文档说明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_parent_findings: Option<bool>,
    /// Fork 上下文继承模式。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork_mode: Option<ForkMode>,
}

/// 解析后的子会话 override 快照。
///
/// 该结构会被事件和状态查询复用，便于调试“最终到底继承了什么”。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedSubagentContextOverrides {
    pub storage_mode: super::SubRunStorageMode,
    pub inherit_system_instructions: bool,
    pub inherit_project_instructions: bool,
    pub inherit_working_dir: bool,
    pub inherit_policy_upper_bound: bool,
    pub inherit_cancel_token: bool,
    pub include_compact_summary: bool,
    pub include_recent_tail: bool,
    pub include_recovery_refs: bool,
    pub include_parent_findings: bool,
    pub fork_mode: Option<ForkMode>,
}

impl Default for ResolvedSubagentContextOverrides {
    fn default() -> Self {
        Self {
            // 默认始终使用独立子会话模式。
            storage_mode: super::SubRunStorageMode::IndependentSession,
            inherit_system_instructions: true,
            inherit_project_instructions: true,
            inherit_working_dir: true,
            inherit_policy_upper_bound: true,
            inherit_cancel_token: true,
            include_compact_summary: false,
            include_recent_tail: true,
            include_recovery_refs: false,
            include_parent_findings: false,
            fork_mode: None,
        }
    }
}

/// 解析后的执行限制快照。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedExecutionLimitsSnapshot {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_steps: Option<u32>,
}

/// child delegation 的轻量元数据。
///
/// 这是 launch / resume / observe 共用的责任连续性投影，
/// 用来描述“这个 child 负责哪条责任分支”以及“复用时要遵守什么边界”。
/// 它不是新的 durable 真相，真正事实仍然来自 lifecycle / turn outcome /
/// resolved capability surface。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DelegationMetadata {
    pub responsibility_summary: String,
    pub reuse_scope_summary: String,
    #[serde(default)]
    pub restricted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_limit_summary: Option<String>,
}

/// Agent 画像定义。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    /// Profile 唯一标识。
    pub id: String,
    /// 人类可读名称。
    pub name: String,
    /// 作用说明，供路由/提示词/UI 复用。
    pub description: String,
    /// 该 profile 允许的使用模式。
    pub mode: AgentMode,
    /// 子 Agent 专用系统提示，可为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// 允许使用的工具集合；为空表示由上层策略决定。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    /// 显式禁止的工具集合。
    ///
    /// 该字段用于保留 Claude 风格 agent 定义里的 denylist 语义，
    /// 即使当前策略层还未完整消费，也不能在加载阶段静默丢失。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disallowed_tools: Vec<String>,
    /// 模型偏好。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_preference: Option<String>,
}

/// 子 Agent profile 目录抽象。
///
/// prompt 组装和执行装配都需要读取当前运行时可见的子 Agent 列表，
/// 因此该 discovery 契约应属于 core 边界，而不是某个具体 tool crate。
pub trait AgentProfileCatalog: Send + Sync {
    fn list_subagent_profiles(&self) -> Vec<AgentProfile>;
}
