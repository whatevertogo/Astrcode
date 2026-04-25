//! # Hooks 合同
//!
//! 定义 hook typed payload/effect 和 dispatch outcome。
//! 作为 `agent-runtime`、`host-session`、`plugin-host` 之间的共享合同类型。
//!
//! ## 设计决策
//!
//! - `HookEffect` 和 `HookEventPayload` 是 typed enum，不在合约层序列化； 协议边界的序列化由
//!   `astrcode-protocol` 的 DTO 负责。
//! - `HookDispatchRequest` 不放在这里，因为不同的 owner 构造 payload 的方式不同； 各 owner 只需分派
//!   `HookEventPayload` 即可。

use std::path::PathBuf;

use astrcode_core::{
    CapabilityKind, CapabilitySpec, CompactTrigger, HookEventKey, InvocationMode, LlmMessage,
    SideEffect, Stability,
};
use serde::Serialize;

// ============================================================================
// HookEventPayload — 每个正式 hook 事件的 typed 输入
// ============================================================================

/// 每个正式 hook 事件的 typed payload。
///
/// 各变体字段由具体的 hook owner 在 dispatch 时填充。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HookEventPayload {
    Input {
        session_id: String,
        source: String,
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<String>,
        current_mode: Option<String>,
    },
    Context {
        session_id: String,
        turn_id: String,
        agent_id: String,
        step_index: usize,
        message_count: usize,
        current_mode: Option<String>,
    },
    BeforeAgentStart {
        session_id: String,
        turn_id: String,
        agent_id: String,
        step_index: usize,
        message_count: usize,
        current_mode: Option<String>,
    },
    BeforeProviderRequest {
        session_id: String,
        turn_id: String,
        provider_ref: String,
        model_ref: String,
        request: serde_json::Value,
        current_mode: Option<String>,
    },
    ToolCall {
        session_id: String,
        turn_id: String,
        agent_id: String,
        tool_call_id: String,
        tool_name: String,
        args: serde_json::Value,
        capability_spec: Box<CapabilitySpec>,
        working_dir: PathBuf,
        current_mode: Option<String>,
        step_index: usize,
    },
    ToolResult {
        session_id: String,
        turn_id: String,
        tool_call_id: String,
        tool_name: String,
        args: serde_json::Value,
        result: serde_json::Value,
        ok: bool,
        current_mode: Option<String>,
    },
    SessionBeforeCompact {
        session_id: String,
        reason: CompactTrigger,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        messages: Vec<LlmMessage>,
        settings: serde_json::Value,
        current_mode: Option<String>,
    },
    ResourcesDiscover {
        snapshot_id: String,
        cwd: PathBuf,
        reason: String,
    },
    ModelSelect {
        session_id: String,
        current_model: String,
        candidate_model: String,
        reason: String,
    },
    TurnStart {
        session_id: String,
        turn_id: String,
        agent_id: String,
        current_mode: Option<String>,
    },
    TurnEnd {
        session_id: String,
        turn_id: String,
        agent_id: String,
        current_mode: Option<String>,
    },
}

// ============================================================================
// HookEffect — hook handler 返回的 typed effect
// ============================================================================

/// Hook handler 返回的 typed effect。
///
/// 每个 variant 的字段按事件约束；owner 应用 effect 前必须校验 effect 属于事件允许集合。
#[derive(Debug, Clone)]
pub enum HookEffect {
    /// 继续执行，不做额外操作。
    Continue,
    /// 记录诊断消息。
    Diagnostic { message: String },
    /// 转换用户输入（仅 `input` 事件）。
    TransformInput { text: String },
    /// 在处理用户输入后不创建 turn（仅 `input` 事件）。
    HandledInput { response: String },
    /// 请求 mode 切换（仅 `input` 事件）。
    SwitchMode { mode_id: String },
    /// 修改 provider request 负载（仅 `before_provider_request` 事件）。
    ModifyProviderRequest { request: serde_json::Value },
    /// 阻止 provider 请求（仅 `before_provider_request` 事件）。
    DenyProviderRequest { reason: String },
    /// 修改工具参数（仅 `tool_call` 事件）。
    MutateToolArgs {
        tool_call_id: String,
        args: serde_json::Value,
    },
    /// 拒绝单个工具调用并生成失败结果（仅 `tool_call` 事件）。
    BlockToolResult {
        tool_call_id: String,
        reason: String,
    },
    /// 需要审批（`tool_call` / `before_provider_request` 事件）。
    RequireApproval { request_id: String, reason: String },
    /// 覆盖工具结果（仅 `tool_result` 事件）。
    OverrideToolResult {
        tool_call_id: String,
        result: serde_json::Value,
        ok: bool,
    },
    /// 取消当前 turn（runtime 可取消事件）。
    CancelTurn { reason: String },
    /// 取消压缩（仅 `session_before_compact` 事件）。
    CancelCompact { reason: String },
    /// 修改压缩输入（仅 `session_before_compact` 事件）。
    OverrideCompactInput {
        reason: CompactTrigger,
        messages: Vec<LlmMessage>,
    },
    /// 提供外部摘要（仅 `session_before_compact` 事件）。
    ProvideCompactSummary { summary: String },
    /// 贡献资源路径（仅 `resources_discover` 事件）。
    ResourcePath { path: String },
    /// 模型建议（仅 `model_select` 事件）。
    ModelHint { model: String },
    /// 拒绝模型切换（仅 `model_select` 事件）。
    DenyModelSelect { reason: String },
}

impl HookEventPayload {
    /// 从 event key + JSON value 构造 typed payload（transitional）。
    ///
    /// 当调用方尚不能构造 typed variant 时，fallback 到通用 `Value` 表示。
    /// Phase 4 后将逐步淘汰这个 fallback。
    pub fn from_value(event: &HookEventKey, value: &serde_json::Value) -> Self {
        let extract = |key: &str| -> String {
            match value.get(key) {
                Some(serde_json::Value::String(s)) => s.clone(),
                _ => String::new(),
            }
        };

        let session_id = extract("sessionId");
        let turn_id = extract("turnId");
        let agent_id = extract("agentId");
        let current_mode = value
            .get("currentMode")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        match event {
            HookEventKey::Input => HookEventPayload::Input {
                session_id,
                source: extract("source"),
                text: extract("text"),
                images: Vec::new(),
                current_mode,
            },
            HookEventKey::Context => HookEventPayload::Context {
                session_id,
                turn_id,
                agent_id,
                step_index: value.get("stepIndex").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                message_count: value
                    .get("messageCount")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize,
                current_mode,
            },
            HookEventKey::BeforeAgentStart => HookEventPayload::BeforeAgentStart {
                session_id,
                turn_id,
                agent_id,
                step_index: value.get("stepIndex").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
                message_count: value
                    .get("messageCount")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize,
                current_mode,
            },
            HookEventKey::BeforeProviderRequest => HookEventPayload::BeforeProviderRequest {
                session_id,
                turn_id,
                provider_ref: extract("providerRef"),
                model_ref: extract("modelRef"),
                request: value.get("request").cloned().unwrap_or_default(),
                current_mode,
            },
            HookEventKey::ToolCall => HookEventPayload::ToolCall {
                session_id,
                turn_id,
                agent_id,
                tool_call_id: extract("toolCallId"),
                tool_name: extract("toolName"),
                args: value.get("args").cloned().unwrap_or_default(),
                capability_spec: Box::new(CapabilitySpec {
                    name: Default::default(),
                    kind: CapabilityKind::Tool,
                    description: String::new(),
                    input_schema: Default::default(),
                    output_schema: Default::default(),
                    invocation_mode: InvocationMode::Unary,
                    concurrency_safe: false,
                    compact_clearable: false,
                    profiles: Vec::new(),
                    tags: Vec::new(),
                    permissions: Vec::new(),
                    side_effect: SideEffect::None,
                    stability: Stability::Stable,
                    metadata: Default::default(),
                    max_result_inline_size: None,
                }),
                working_dir: PathBuf::new(),
                current_mode,
                step_index: value.get("stepIndex").and_then(|v| v.as_u64()).unwrap_or(0) as usize,
            },
            HookEventKey::ToolResult => HookEventPayload::ToolResult {
                session_id,
                turn_id,
                tool_call_id: extract("toolCallId"),
                tool_name: extract("toolName"),
                args: value.get("args").cloned().unwrap_or_default(),
                result: value.get("result").cloned().unwrap_or_default(),
                ok: value.get("ok").and_then(|v| v.as_bool()).unwrap_or(true),
                current_mode,
            },
            HookEventKey::SessionBeforeCompact => HookEventPayload::SessionBeforeCompact {
                session_id,
                reason: CompactTrigger::Auto,
                messages: Vec::new(),
                settings: value.clone(),
                current_mode,
            },
            HookEventKey::ResourcesDiscover => HookEventPayload::ResourcesDiscover {
                snapshot_id: session_id,
                cwd: PathBuf::new(),
                reason: extract("reason"),
            },
            HookEventKey::ModelSelect => HookEventPayload::ModelSelect {
                session_id,
                current_model: extract("currentModel"),
                candidate_model: extract("candidateModel"),
                reason: extract("reason"),
            },
            HookEventKey::TurnStart => HookEventPayload::TurnStart {
                session_id,
                turn_id,
                agent_id,
                current_mode,
            },
            HookEventKey::TurnEnd => HookEventPayload::TurnEnd {
                session_id,
                turn_id,
                agent_id,
                current_mode,
            },
        }
    }

    pub fn event_key(&self) -> HookEventKey {
        match self {
            HookEventPayload::Input { .. } => HookEventKey::Input,
            HookEventPayload::Context { .. } => HookEventKey::Context,
            HookEventPayload::BeforeAgentStart { .. } => HookEventKey::BeforeAgentStart,
            HookEventPayload::BeforeProviderRequest { .. } => HookEventKey::BeforeProviderRequest,
            HookEventPayload::ToolCall { .. } => HookEventKey::ToolCall,
            HookEventPayload::ToolResult { .. } => HookEventKey::ToolResult,
            HookEventPayload::SessionBeforeCompact { .. } => HookEventKey::SessionBeforeCompact,
            HookEventPayload::ResourcesDiscover { .. } => HookEventKey::ResourcesDiscover,
            HookEventPayload::ModelSelect { .. } => HookEventKey::ModelSelect,
            HookEventPayload::TurnStart { .. } => HookEventKey::TurnStart,
            HookEventPayload::TurnEnd { .. } => HookEventKey::TurnEnd,
        }
    }

    pub fn current_mode(&self) -> Option<&str> {
        match self {
            HookEventPayload::Input { current_mode, .. }
            | HookEventPayload::Context { current_mode, .. }
            | HookEventPayload::BeforeAgentStart { current_mode, .. }
            | HookEventPayload::BeforeProviderRequest { current_mode, .. }
            | HookEventPayload::ToolCall { current_mode, .. }
            | HookEventPayload::ToolResult { current_mode, .. }
            | HookEventPayload::SessionBeforeCompact { current_mode, .. }
            | HookEventPayload::TurnStart { current_mode, .. }
            | HookEventPayload::TurnEnd { current_mode, .. } => current_mode.as_deref(),
            HookEventPayload::ResourcesDiscover { .. } | HookEventPayload::ModelSelect { .. } => {
                None
            },
        }
    }
}

impl HookEffect {
    /// 返回 true 表示该 effect 会终止当前流程。
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            HookEffect::CancelTurn { .. }
                | HookEffect::DenyProviderRequest { .. }
                | HookEffect::DenyModelSelect { .. }
                | HookEffect::CancelCompact { .. }
        )
    }

    /// 返回 true 表示该 effect 允许流程继续。
    pub fn is_continue(&self) -> bool {
        matches!(self, HookEffect::Continue)
    }
}

// ============================================================================
// HookDispatchOutcome — hook dispatch 结果
// ============================================================================

/// Hook dispatch 结果：所有匹配 handler 返回的 effect 集合。
#[derive(Debug, Clone, Default)]
pub struct HookDispatchOutcome {
    pub effects: Vec<HookEffect>,
}

impl HookDispatchOutcome {
    pub fn empty() -> Self {
        Self {
            effects: Vec::new(),
        }
    }
}

/// 获取事件允许的 effect 集合（用于校验）。
pub fn allowed_effects_for_event(event: &str) -> &[&str] {
    match event {
        "input" => &[
            "Continue",
            "Diagnostic",
            "TransformInput",
            "HandledInput",
            "SwitchMode",
        ],
        "context" => &["Continue", "Diagnostic"],
        "before_agent_start" => &["Continue", "Diagnostic"],
        "before_provider_request" => &[
            "Continue",
            "Diagnostic",
            "ModifyProviderRequest",
            "DenyProviderRequest",
            "RequireApproval",
            "CancelTurn",
        ],
        "tool_call" => &[
            "Continue",
            "Diagnostic",
            "MutateToolArgs",
            "BlockToolResult",
            "RequireApproval",
            "CancelTurn",
        ],
        "tool_result" => &["Continue", "Diagnostic", "OverrideToolResult"],
        "turn_start" => &["Continue", "Diagnostic"],
        "turn_end" => &["Continue", "Diagnostic"],
        "session_before_compact" => &[
            "Continue",
            "Diagnostic",
            "CancelCompact",
            "OverrideCompactInput",
            "ProvideCompactSummary",
        ],
        "resources_discover" => &["Continue", "Diagnostic", "ResourcePath"],
        "model_select" => &["Continue", "Diagnostic", "ModelHint", "DenyModelSelect"],
        _ => &["Continue", "Diagnostic"],
    }
}
