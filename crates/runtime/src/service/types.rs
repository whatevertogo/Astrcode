//! # 服务类型定义 (Service Types)
//!
//! 定义 `RuntimeService` 的公共类型，包括：
//! - 错误类型（`ServiceError`）及其 HTTP 状态码映射
//! - 会话回放相关类型（`SessionReplay`、`SessionReplaySource`）
//! - 会话目录事件（`SessionCatalogEvent`）
//! - Prompt 接受确认（`PromptAccepted`）
//!
//! ## 错误映射策略
//!
//! `ServiceError` 的每个变体对应一个 HTTP 状态码类别：
//! - `NotFound` → 404
//! - `Conflict` → 409
//! - `InvalidInput` → 400
//! - `Internal` → 500
//!
//! 错误从底层（`AstrError`、`StoreError`）向上传递时，
//! 通过 `From` trait 自动映射到对应的 HTTP 语义类别。

use std::fmt::{Display, Formatter};

use astrcode_core::{AstrError, Phase, StoreError};
pub use astrcode_core::{SessionEventRecord, SessionMessage};
use async_trait::async_trait;
use tokio::sync::broadcast;

/// Prompt 提交成功的响应
///
/// 表示用户的 Prompt 已被接受并分配了 Turn ID。
/// 如果会话是从另一个会话分支出来的，`branched_from_session_id` 会记录源会话。
#[derive(Debug, Clone)]
pub struct PromptAccepted {
    /// 本次 Turn 的唯一标识
    pub turn_id: String,
    /// 目标会话 ID
    pub session_id: String,
    /// 如果是分支会话，记录源会话 ID
    pub branched_from_session_id: Option<String>,
}

/// 会话回放结果
///
/// 包含历史事件记录和实时事件订阅者。
/// 前端可以先消费 `history` 回放历史，然后切换到 `receiver` 接收实时事件。
pub struct SessionReplay {
    /// 历史事件记录（从 `last_event_id` 之后开始）
    pub history: Vec<SessionEventRecord>,
    /// 实时事件订阅者，用于接收后续新事件
    pub receiver: broadcast::Receiver<SessionEventRecord>,
}

/// 会话历史快照。
///
/// 为前端初始化会话提供单一协议面：历史 `AgentEvent`、最新游标和当前 phase。
/// 这样初始 hydration 和后续 SSE 增量都基于同一事件模型，不再维护第二套
/// `SessionMessage` 专用传输协议。
#[derive(Debug, Clone)]
pub struct SessionHistorySnapshot {
    pub history: Vec<SessionEventRecord>,
    pub cursor: Option<String>,
    pub phase: Phase,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionCatalogEvent {
    SessionCreated {
        session_id: String,
    },
    SessionDeleted {
        session_id: String,
    },
    ProjectDeleted {
        working_dir: String,
    },
    SessionBranched {
        session_id: String,
        source_session_id: String,
    },
}

#[async_trait]
pub trait SessionReplaySource {
    async fn replay(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> ServiceResult<SessionReplay>;
}

#[derive(Debug)]
pub enum ServiceError {
    NotFound(String),
    Conflict(String),
    InvalidInput(String),
    Internal(AstrError),
}

impl Display for ServiceError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(message) | Self::Conflict(message) | Self::InvalidInput(message) => {
                f.write_str(message)
            },
            Self::Internal(error) => Display::fmt(error, f),
        }
    }
}

impl std::error::Error for ServiceError {}

impl From<anyhow::Error> for ServiceError {
    fn from(value: anyhow::Error) -> Self {
        // 两级 downcast 链：spawn_blocking_service 将错误包装为 anyhow::Error
        // 传输（因为 tokio::task::spawn_blocking 返回 JoinError + 闭包返回值
        // 需要类型擦除）。此链尝试恢复原始错误变体以正确映射 HTTP 状态码：
        // 1. 先尝试还原为 ServiceError（跨越 spawn_blocking 边界的业务错误）
        // 2. 再尝试还原为 AstrError（底层领域错误）
        // 3. 都失败则包装为 Internal
        let value = match value.downcast::<ServiceError>() {
            Ok(service_error) => return service_error,
            Err(value) => value,
        };
        match value.downcast::<AstrError>() {
            Ok(astr_error) => Self::from(astr_error),
            Err(other) => Self::Internal(AstrError::Internal(other.to_string())),
        }
    }
}

/// 将领域错误映射为 HTTP 语义错误。
///
/// 每个 AstrError 变体被归类到对应的 HTTP 状态码类别：
/// - NotFound (404): SessionNotFound, ProjectNotFound
/// - Conflict (409): TurnInProgress
/// - InvalidInput (400): Validation, InvalidSessionId, MissingApiKey, MissingBaseUrl
/// - Internal (500): 其他所有错误（IO、LLM 失败等）
impl From<AstrError> for ServiceError {
    fn from(value: AstrError) -> Self {
        match &value {
            AstrError::SessionNotFound(id) => Self::NotFound(format!("session not found: {}", id)),
            AstrError::ProjectNotFound(id) => Self::NotFound(format!("project not found: {}", id)),
            AstrError::TurnInProgress(id) => {
                Self::Conflict(format!("turn already in progress: {}", id))
            },
            AstrError::Validation(msg) => Self::InvalidInput(msg.clone()),
            AstrError::InvalidSessionId(id) => {
                Self::InvalidInput(format!("invalid session id: {}", id))
            },
            AstrError::MissingApiKey(profile) => {
                Self::InvalidInput(format!("missing api key for profile: {}", profile))
            },
            AstrError::MissingBaseUrl(profile) => {
                Self::InvalidInput(format!("missing base url for profile: {}", profile))
            },
            _ => Self::Internal(value),
        }
    }
}

impl From<StoreError> for ServiceError {
    fn from(value: StoreError) -> Self {
        match value {
            StoreError::SessionNotFound(id) => Self::NotFound(format!("session not found: {}", id)),
            StoreError::InvalidSessionId(id) => {
                Self::InvalidInput(format!("invalid session id: {}", id))
            },
            StoreError::Io { context, .. } => Self::Internal(AstrError::Internal(context)),
            StoreError::Parse { context, .. } => Self::Internal(AstrError::Internal(context)),
        }
    }
}

pub type ServiceResult<T> = std::result::Result<T, ServiceError>;

/// 输入候选的语义类型。
///
/// 这里保持为 runtime 内部类型，server 再投影到 HTTP DTO，
/// 这样 service 层不需要直接依赖 HTTP 传输细节。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComposerOptionKind {
    Command,
    Skill,
    Capability,
}

/// 输入候选查询参数。
///
/// `query` 和 `kinds` 都是可选的：前端可以先取默认推荐列表，
/// 再在本地继续交互式缩小范围。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerOptionsRequest {
    pub query: Option<String>,
    pub kinds: Vec<ComposerOptionKind>,
    pub limit: usize,
}

impl Default for ComposerOptionsRequest {
    fn default() -> Self {
        Self {
            query: None,
            kinds: Vec::new(),
            limit: 50,
        }
    }
}

/// 单个输入候选项。
///
/// `insert_text` 明确写入 service 结果里，是为了把“候选展示”和“选中后回填”
/// 这两个动作绑定到同一份后端语义，而不是让前端自己猜。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerOption {
    pub kind: ComposerOptionKind,
    pub id: String,
    pub title: String,
    pub description: String,
    pub insert_text: String,
    pub badges: Vec<String>,
    pub keywords: Vec<String>,
}
