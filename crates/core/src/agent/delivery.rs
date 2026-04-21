use serde::{Deserialize, Serialize};

use super::lifecycle::{AgentLifecycleStatus, AgentTurnOutcome};

/// 子会话事件写入的存储模式。
///
/// TODO: 当前只有 `IndependentSession` 一个变体。
/// 如果未来真的要支持共享 session / 嵌套持久化域等模式，再扩展枚举；
/// 在那之前保留 enum 形状，避免过早把潜在扩展点压成单态值对象。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SubRunStorageMode {
    /// 使用独立 child session。
    IndependentSession,
}

/// 子执行输出引用。
///
/// 这里只做轻量引用，不在本轮引入重量级 artifact 平台，
/// 避免把“子会话语义”实现膨胀成“产物管理系统”。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRef {
    pub kind: String,
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
}

/// 子执行失败分类。
///
/// 这里使用稳定枚举而不是裸字符串，避免前后端各自维护一套错误码字面量。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubRunFailureCode {
    Transport,
    ProviderHttp,
    StreamParse,
    Interrupted,
    Internal,
}

/// child -> parent 的 typed delivery 分类。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParentDeliveryKind {
    Progress,
    Completed,
    Failed,
    CloseRequest,
}

/// child -> parent delivery 的来源。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParentDeliveryOrigin {
    Explicit,
    Fallback,
}

/// delivery 是否终结当前 child work turn。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ParentDeliveryTerminalSemantics {
    NonTerminal,
    Terminal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProgressParentDeliveryPayload {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompletedParentDeliveryPayload {
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FailedParentDeliveryPayload {
    pub message: String,
    pub code: SubRunFailureCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub technical_message: Option<String>,
    pub retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CloseRequestParentDeliveryPayload {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// child -> parent 的结构化 payload。
///
/// 使用判别联合而不是无结构 blob，防止 contract 退化回“只有 kind + 文本”。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum ParentDeliveryPayload {
    Progress(ProgressParentDeliveryPayload),
    Completed(CompletedParentDeliveryPayload),
    Failed(FailedParentDeliveryPayload),
    CloseRequest(CloseRequestParentDeliveryPayload),
}

impl ParentDeliveryPayload {
    pub fn kind(&self) -> ParentDeliveryKind {
        match self {
            Self::Progress(_) => ParentDeliveryKind::Progress,
            Self::Completed(_) => ParentDeliveryKind::Completed,
            Self::Failed(_) => ParentDeliveryKind::Failed,
            Self::CloseRequest(_) => ParentDeliveryKind::CloseRequest,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Progress(payload) => payload.message.as_str(),
            Self::Completed(payload) => payload.message.as_str(),
            Self::Failed(payload) => payload.message.as_str(),
            Self::CloseRequest(payload) => payload.message.as_str(),
        }
    }
}

/// child -> parent 的 typed delivery envelope。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ParentDelivery {
    pub idempotency_key: String,
    pub origin: ParentDeliveryOrigin,
    pub terminal_semantics: ParentDeliveryTerminalSemantics,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_turn_id: Option<String>,
    #[serde(flatten)]
    pub payload: ParentDeliveryPayload,
}

/// 子执行传递给父会话的业务结果。
///
/// 该结构只承载“父 Agent 后续决策真正需要消费的内容”，
/// 明确排除 transport/provider/internal diagnostics。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct SubRunHandoff {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<ParentDelivery>,
}

/// 子执行失败的结构化信息。
///
/// `display_message` 面向父 Agent / UI 主视图，要求短且稳定；
/// `technical_message` 仅用于调试与次级展示，不应直接进入父会话 handoff。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubRunFailure {
    pub code: SubRunFailureCode,
    pub display_message: String,
    pub technical_message: String,
    pub retryable: bool,
}

/// 子执行结构化结果。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompletedSubRunOutcome {
    Completed,
    TokenExceeded,
}

impl CompletedSubRunOutcome {
    pub fn as_turn_outcome(self) -> AgentTurnOutcome {
        match self {
            Self::Completed => AgentTurnOutcome::Completed,
            Self::TokenExceeded => AgentTurnOutcome::TokenExceeded,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailedSubRunOutcome {
    Failed,
    Cancelled,
}

impl FailedSubRunOutcome {
    pub fn as_turn_outcome(self) -> AgentTurnOutcome {
        match self {
            Self::Failed => AgentTurnOutcome::Failed,
            Self::Cancelled => AgentTurnOutcome::Cancelled,
        }
    }
}

/// 子执行对外可观察的正式状态。
///
/// 这是 `SubRunResult` 的 canonical status projection，避免外围再组合
/// `lifecycle + last_turn_outcome` 反推业务语义。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubRunStatus {
    Running,
    Completed,
    TokenExceeded,
    Failed,
    Cancelled,
}

impl SubRunStatus {
    pub fn lifecycle(self) -> AgentLifecycleStatus {
        match self {
            Self::Running => AgentLifecycleStatus::Running,
            Self::Completed | Self::TokenExceeded | Self::Failed | Self::Cancelled => {
                AgentLifecycleStatus::Idle
            },
        }
    }

    pub fn last_turn_outcome(self) -> Option<AgentTurnOutcome> {
        match self {
            Self::Running => None,
            Self::Completed => Some(AgentTurnOutcome::Completed),
            Self::TokenExceeded => Some(AgentTurnOutcome::TokenExceeded),
            Self::Failed => Some(AgentTurnOutcome::Failed),
            Self::Cancelled => Some(AgentTurnOutcome::Cancelled),
        }
    }

    pub fn is_failed(self) -> bool {
        matches!(self, Self::Failed)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::TokenExceeded => "token_exceeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SubRunResult {
    Running {
        handoff: SubRunHandoff,
    },
    Completed {
        outcome: CompletedSubRunOutcome,
        handoff: SubRunHandoff,
    },
    Failed {
        outcome: FailedSubRunOutcome,
        failure: SubRunFailure,
    },
}

impl SubRunResult {
    pub fn status(&self) -> SubRunStatus {
        match self {
            Self::Running { .. } => SubRunStatus::Running,
            Self::Completed { outcome, .. } => match outcome {
                CompletedSubRunOutcome::Completed => SubRunStatus::Completed,
                CompletedSubRunOutcome::TokenExceeded => SubRunStatus::TokenExceeded,
            },
            Self::Failed { outcome, .. } => match outcome {
                FailedSubRunOutcome::Failed => SubRunStatus::Failed,
                FailedSubRunOutcome::Cancelled => SubRunStatus::Cancelled,
            },
        }
    }

    pub fn lifecycle(&self) -> AgentLifecycleStatus {
        self.status().lifecycle()
    }

    pub fn last_turn_outcome(&self) -> Option<AgentTurnOutcome> {
        self.status().last_turn_outcome()
    }

    pub fn handoff(&self) -> Option<&SubRunHandoff> {
        match self {
            Self::Running { handoff } | Self::Completed { handoff, .. } => Some(handoff),
            Self::Failed { .. } => None,
        }
    }

    pub fn failure(&self) -> Option<&SubRunFailure> {
        match self {
            Self::Failed { failure, .. } => Some(failure),
            Self::Running { .. } | Self::Completed { .. } => None,
        }
    }

    pub fn is_failed(&self) -> bool {
        self.status().is_failed()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CompletedSubRunOutcome, FailedSubRunOutcome, SubRunFailure, SubRunFailureCode,
        SubRunHandoff, SubRunResult, SubRunStatus,
    };
    use crate::agent::lifecycle::{AgentLifecycleStatus, AgentTurnOutcome};

    fn sample_handoff() -> SubRunHandoff {
        SubRunHandoff {
            findings: vec!["done".to_string()],
            artifacts: Vec::new(),
            delivery: None,
        }
    }

    fn sample_failure() -> SubRunFailure {
        SubRunFailure {
            code: SubRunFailureCode::Internal,
            display_message: "failed".to_string(),
            technical_message: "stack".to_string(),
            retryable: false,
        }
    }

    #[test]
    fn subrun_status_methods_cover_all_variants() {
        let cases = [
            (
                SubRunStatus::Running,
                AgentLifecycleStatus::Running,
                None,
                false,
                "running",
            ),
            (
                SubRunStatus::Completed,
                AgentLifecycleStatus::Idle,
                Some(AgentTurnOutcome::Completed),
                false,
                "completed",
            ),
            (
                SubRunStatus::TokenExceeded,
                AgentLifecycleStatus::Idle,
                Some(AgentTurnOutcome::TokenExceeded),
                false,
                "token_exceeded",
            ),
            (
                SubRunStatus::Failed,
                AgentLifecycleStatus::Idle,
                Some(AgentTurnOutcome::Failed),
                true,
                "failed",
            ),
            (
                SubRunStatus::Cancelled,
                AgentLifecycleStatus::Idle,
                Some(AgentTurnOutcome::Cancelled),
                false,
                "cancelled",
            ),
        ];

        for (status, expected_lifecycle, expected_outcome, expected_failed, expected_label) in cases
        {
            assert_eq!(status.lifecycle(), expected_lifecycle);
            assert_eq!(status.last_turn_outcome(), expected_outcome);
            assert_eq!(status.is_failed(), expected_failed);
            assert_eq!(status.label(), expected_label);
        }
    }

    #[test]
    fn subrun_result_methods_project_structured_state() {
        let handoff = sample_handoff();
        let running = SubRunResult::Running {
            handoff: handoff.clone(),
        };
        assert_eq!(running.status(), SubRunStatus::Running);
        assert_eq!(running.lifecycle(), AgentLifecycleStatus::Running);
        assert_eq!(running.last_turn_outcome(), None);
        assert_eq!(running.handoff(), Some(&handoff));
        assert_eq!(running.failure(), None);
        assert!(!running.is_failed());

        let completed = SubRunResult::Completed {
            outcome: CompletedSubRunOutcome::Completed,
            handoff: handoff.clone(),
        };
        assert_eq!(completed.status(), SubRunStatus::Completed);
        assert_eq!(completed.lifecycle(), AgentLifecycleStatus::Idle);
        assert_eq!(
            completed.last_turn_outcome(),
            Some(AgentTurnOutcome::Completed)
        );
        assert_eq!(completed.handoff(), Some(&handoff));
        assert_eq!(completed.failure(), None);
        assert!(!completed.is_failed());

        let token_exceeded = SubRunResult::Completed {
            outcome: CompletedSubRunOutcome::TokenExceeded,
            handoff,
        };
        assert_eq!(token_exceeded.status(), SubRunStatus::TokenExceeded);
        assert_eq!(
            token_exceeded.last_turn_outcome(),
            Some(AgentTurnOutcome::TokenExceeded)
        );

        let failure = sample_failure();
        let failed = SubRunResult::Failed {
            outcome: FailedSubRunOutcome::Failed,
            failure: failure.clone(),
        };
        assert_eq!(failed.status(), SubRunStatus::Failed);
        assert_eq!(failed.lifecycle(), AgentLifecycleStatus::Idle);
        assert_eq!(failed.last_turn_outcome(), Some(AgentTurnOutcome::Failed));
        assert_eq!(failed.handoff(), None);
        assert_eq!(failed.failure(), Some(&failure));
        assert!(failed.is_failed());

        let cancelled = SubRunResult::Failed {
            outcome: FailedSubRunOutcome::Cancelled,
            failure,
        };
        assert_eq!(cancelled.status(), SubRunStatus::Cancelled);
        assert_eq!(
            cancelled.last_turn_outcome(),
            Some(AgentTurnOutcome::Cancelled)
        );
        assert!(!cancelled.is_failed());
    }
}
