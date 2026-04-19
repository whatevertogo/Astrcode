//! # 执行结果公共字段
//!
//! 提取工具结果与能力结果中共享的 `error / metadata / duration_ms / truncated` 字段，
//! 避免两套结果类型平行复制相同字段。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ChildAgentRef;

/// 执行结果中与调用类型无关的公共字段。
///
/// 该结构只承载通用执行元数据，避免工具结果与能力结果继续平行复制
/// `error / metadata / duration_ms / truncated` 四组字段。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionResultCommon {
    /// 错误信息（仅在失败时设置）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 额外元数据（如 diff 信息、终端显示提示等）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    /// 执行耗时（毫秒）
    pub duration_ms: u64,
    /// 输出是否因大小限制被截断
    #[serde(default)]
    pub truncated: bool,
}

impl ExecutionResultCommon {
    pub fn success(metadata: Option<Value>, duration_ms: u64, truncated: bool) -> Self {
        Self {
            error: None,
            metadata,
            duration_ms,
            truncated,
        }
    }

    pub fn failure(
        error: impl Into<String>,
        metadata: Option<Value>,
        duration_ms: u64,
        truncated: bool,
    ) -> Self {
        Self {
            error: Some(error.into()),
            metadata,
            duration_ms,
            truncated,
        }
    }
}

/// 执行结果暴露给下游消费者的 typed 续接目标。
///
/// Why:
/// - 这不是所有执行结果共享的横切公共字段，因此不属于 `ExecutionResultCommon`
/// - 也不应退回弱类型 `metadata`
/// - 统一承载 spawn/send/observe/close 产生的后续协作续接语义
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
// TODO: 目前只有 child agent 一种续接目标，但未来可以扩展为工具调用、能力调用等。
pub enum ExecutionContinuation {
    ChildAgent { child_ref: ChildAgentRef },
}

impl ExecutionContinuation {
    pub fn child_agent(child_ref: ChildAgentRef) -> Self {
        Self::ChildAgent { child_ref }
    }

    pub fn child_agent_ref(&self) -> Option<&ChildAgentRef> {
        match self {
            Self::ChildAgent { child_ref } => Some(child_ref),
        }
    }
}
