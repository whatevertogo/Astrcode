//! # 能力路由契约
//!
//! core 仅保留能力调用相关的契约与 DTO，具体路由实现下沉到 runtime-registry。

use std::{fmt, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    AgentEventContext, CancelToken, CapabilityDescriptor, ExecutionOwner, Result, ToolContext,
    ToolEventSink, ToolExecutionResult, ToolOutputDelta,
};

/// 能力调用的上下文信息。
#[derive(Clone)]
pub struct CapabilityContext {
    /// 请求唯一标识，用于追踪单次调用链路
    pub request_id: Option<String>,
    /// 分布式追踪标识，关联同一请求的多个子操作
    pub trace_id: Option<String>,
    /// 所属会话标识
    pub session_id: String,
    /// 工作目录，工具执行时的当前路径
    pub working_dir: PathBuf,
    /// 取消令牌，用于外部中断长时间运行的能力调用
    pub cancel: CancelToken,
    /// 当前调用所属 turn。
    pub turn_id: Option<String>,
    /// 当前调用所属 Agent 元数据。
    pub agent: AgentEventContext,
    /// 当前调用所属执行 owner。
    pub execution_owner: Option<ExecutionOwner>,
    /// 当前使用的 profile 名称
    pub profile: String,
    /// profile 上下文，包含工作目录、仓库根目录等运行时配置
    pub profile_context: Value,
    /// 调用方自定义元数据
    pub metadata: Value,
    /// 工具增量输出发送器，用于流式推送工具执行结果
    pub tool_output_sender: Option<UnboundedSender<ToolOutputDelta>>,
    /// 工具内部 turn 事件发射器。
    pub event_sink: Option<Arc<dyn ToolEventSink>>,
}

impl fmt::Debug for CapabilityContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CapabilityContext")
            .field("request_id", &self.request_id)
            .field("trace_id", &self.trace_id)
            .field("session_id", &self.session_id)
            .field("working_dir", &self.working_dir)
            .field("cancel", &self.cancel)
            .field("turn_id", &self.turn_id)
            .field("agent", &self.agent)
            .field("execution_owner", &self.execution_owner)
            .field("profile", &self.profile)
            .field("profile_context", &self.profile_context)
            .field("metadata", &self.metadata)
            .field(
                "tool_output_sender",
                &self.tool_output_sender.as_ref().map(|_| "<attached>"),
            )
            .field(
                "event_sink",
                &self.event_sink.as_ref().map(|_| "<attached>"),
            )
            .finish()
    }
}

impl CapabilityContext {
    pub fn from_tool_context(ctx: &ToolContext, request_id: impl Into<Option<String>>) -> Self {
        // 只分配一次：先获取 PathBuf，再从中提取字符串用于 profile_context
        let working_dir = ctx.working_dir().to_path_buf();
        let working_dir_str = working_dir.to_string_lossy().into_owned();
        Self {
            request_id: request_id.into(),
            trace_id: None,
            session_id: ctx.session_id().to_string(),
            working_dir,
            cancel: ctx.cancel().clone(),
            turn_id: ctx.turn_id().map(ToString::to_string),
            agent: ctx.agent_context().clone(),
            execution_owner: ctx.execution_owner().cloned(),
            profile: "coding".to_string(),
            profile_context: json!({
                "workingDir": working_dir_str,
                "repoRoot": working_dir_str,
                "approvalMode": "inherit"
            }),
            metadata: Value::Null,
            tool_output_sender: ctx.tool_output_sender(),
            event_sink: ctx.event_sink(),
        }
    }
}

/// 能力执行结果。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityExecutionResult {
    /// 能力名称
    pub capability_name: String,
    /// 是否执行成功
    pub success: bool,
    /// 执行输出（JSON 值）
    pub output: Value,
    /// 错误信息（仅在失败时设置）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 执行元数据，如 diff 信息、终端输出类型等
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    /// 执行耗时（毫秒）
    pub duration_ms: u64,
    /// 输出是否被截断
    #[serde(default)]
    pub truncated: bool,
}

impl CapabilityExecutionResult {
    /// 构造成功结果。
    pub fn ok(capability_name: impl Into<String>, output: Value) -> Self {
        Self {
            capability_name: capability_name.into(),
            success: true,
            output,
            error: None,
            metadata: None,
            duration_ms: 0,
            truncated: false,
        }
    }

    /// 构造失败结果。
    pub fn failure(
        capability_name: impl Into<String>,
        error: impl Into<String>,
        output: Value,
    ) -> Self {
        Self {
            capability_name: capability_name.into(),
            success: false,
            output,
            error: Some(error.into()),
            metadata: None,
            duration_ms: 0,
            truncated: false,
        }
    }

    /// 将输出格式化为可读文本。
    ///
    /// 字符串直接返回，其他类型序列化为 pretty JSON。
    pub fn output_text(&self) -> String {
        match &self.output {
            Value::Null => String::new(),
            Value::String(text) => text.clone(),
            other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
        }
    }

    /// 转换为 LLM 工具执行结果。
    ///
    /// 将通用的能力执行结果映射为 `ToolExecutionResult`，
    /// 以便前端渲染工具调用卡片。
    pub fn into_tool_execution_result(self, tool_call_id: String) -> ToolExecutionResult {
        let output = self.output_text();
        ToolExecutionResult {
            tool_call_id,
            tool_name: self.capability_name,
            ok: self.success,
            output,
            error: self.error,
            metadata: self.metadata,
            duration_ms: self.duration_ms,
            truncated: self.truncated,
        }
    }
}

/// 能力调用器 trait。
///
/// 所有能力执行器必须实现此 trait，路由器通过它统一分派调用。
#[async_trait]
pub trait CapabilityInvoker: Send + Sync {
    /// 获取能力描述符，包含名称、类型、输入 schema 等元信息。
    fn descriptor(&self) -> CapabilityDescriptor;

    /// 执行能力调用。
    ///
    /// `payload` 为调用参数，`ctx` 携带调用上下文。
    async fn invoke(
        &self,
        payload: Value,
        ctx: &CapabilityContext,
    ) -> Result<CapabilityExecutionResult>;
}
