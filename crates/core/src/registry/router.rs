//! # 能力路由契约
//!
//! core 仅保留能力调用相关的契约与 DTO，具体路由实现下沉到 adapter 层

use std::{fmt, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    AgentEventContext, BoundModeToolContractSnapshot, CancelToken, CapabilitySpec,
    ExecutionContinuation, ExecutionOwner, ExecutionResultCommon, ModeId, Result, SessionId,
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
    pub session_id: SessionId,
    /// 工作目录，工具执行时的当前路径
    pub working_dir: PathBuf,
    /// 取消令牌，用于外部中断长时间运行的能力调用
    pub cancel: CancelToken,
    /// 当前调用所属 turn。
    pub turn_id: Option<String>,
    /// 当前调用所属 Agent 元数据。
    pub agent: AgentEventContext,
    /// 当前调用开始时的治理 mode。
    pub current_mode_id: ModeId,
    /// 当前 turn 绑定后的 mode tool contract 快照。
    pub bound_mode_tool_contract: Option<BoundModeToolContractSnapshot>,
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
            .field("current_mode_id", &self.current_mode_id)
            .field("bound_mode_tool_contract", &self.bound_mode_tool_contract)
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
    /// 能力结果产生的 typed 续接目标。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<ExecutionContinuation>,
    /// 执行耗时（毫秒）
    pub duration_ms: u64,
    /// 输出是否被截断
    #[serde(default)]
    pub truncated: bool,
}

impl CapabilityExecutionResult {
    /// 用公共执行结果字段一次性构造能力结果，避免二段式覆盖。
    pub fn from_common(
        capability_name: impl Into<String>,
        success: bool,
        output: Value,
        continuation: Option<ExecutionContinuation>,
        common: ExecutionResultCommon,
    ) -> Self {
        Self {
            capability_name: capability_name.into(),
            success,
            output,
            error: common.error,
            metadata: common.metadata,
            continuation,
            duration_ms: common.duration_ms,
            truncated: common.truncated,
        }
    }

    /// 构造成功结果。
    pub fn ok(capability_name: impl Into<String>, output: Value) -> Self {
        Self {
            capability_name: capability_name.into(),
            success: true,
            output,
            error: None,
            metadata: None,
            continuation: None,
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
            continuation: None,
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

    /// 将通用能力执行结果转换为 LLM 工具执行结果。
    ///
    /// 填充 tool_call_id 并将 JSON 输出序列化为可读文本，
    /// 使结果能直接用于前端工具卡片渲染和 LLM 上下文回传。
    pub fn into_tool_execution_result(self, tool_call_id: String) -> ToolExecutionResult {
        let output = self.output_text();
        ToolExecutionResult {
            tool_call_id,
            tool_name: self.capability_name,
            ok: self.success,
            output,
            error: self.error,
            metadata: self.metadata,
            continuation: self.continuation,
            duration_ms: self.duration_ms,
            truncated: self.truncated,
        }
    }

    pub fn continuation(&self) -> Option<&ExecutionContinuation> {
        self.continuation.as_ref()
    }

    pub fn common(&self) -> ExecutionResultCommon {
        ExecutionResultCommon {
            error: self.error.clone(),
            metadata: self.metadata.clone(),
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
    /// 获取能力规范，包含名称、类型、输入 schema 等元信息。
    fn capability_spec(&self) -> CapabilitySpec;

    /// 执行能力调用。
    ///
    /// `payload` 为调用参数，`ctx` 携带调用上下文。
    async fn invoke(
        &self,
        payload: Value,
        ctx: &CapabilityContext,
    ) -> Result<CapabilityExecutionResult>;
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::CapabilityExecutionResult;
    use crate::{
        AgentLifecycleStatus, ChildAgentRef, ChildExecutionIdentity, ChildSessionLineageKind,
        ExecutionContinuation, ExecutionResultCommon, ParentExecutionRef,
    };

    #[test]
    fn from_common_preserves_failure_fields_without_placeholder_override() {
        let result = CapabilityExecutionResult::from_common(
            "plugin.read",
            false,
            json!(null),
            None,
            ExecutionResultCommon::failure(
                "transport failed",
                Some(json!({ "streamEvents": [] })),
                23,
                false,
            ),
        );

        assert!(!result.success);
        assert_eq!(result.error.as_deref(), Some("transport failed"));
        assert_eq!(result.metadata, Some(json!({ "streamEvents": [] })));
        assert_eq!(result.duration_ms, 23);
        assert!(!result.truncated);
    }

    #[test]
    fn into_tool_execution_result_preserves_child_continuation() {
        let child_ref = ChildAgentRef {
            identity: ChildExecutionIdentity {
                agent_id: "agent-child".into(),
                session_id: "session-parent".into(),
                sub_run_id: "subrun-1".into(),
            },
            parent: ParentExecutionRef {
                parent_agent_id: Some("agent-parent".into()),
                parent_sub_run_id: Some("subrun-parent".into()),
            },
            lineage_kind: ChildSessionLineageKind::Spawn,
            status: AgentLifecycleStatus::Running,
            open_session_id: "session-child".into(),
        };
        let result = CapabilityExecutionResult::from_common(
            "spawn",
            true,
            json!("spawn accepted"),
            Some(ExecutionContinuation::child_agent(child_ref.clone())),
            ExecutionResultCommon::success(None, 17, false),
        );

        let tool_result = result.into_tool_execution_result("call-1".to_string());
        assert_eq!(
            tool_result.continuation,
            Some(ExecutionContinuation::child_agent(child_ref))
        );
        assert_eq!(tool_result.duration_ms, 17);
    }
}
