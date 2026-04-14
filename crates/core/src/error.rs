//! # 统一错误类型
//!
//! 定义项目级错误枚举 `AstrError`，覆盖会话、配置、工具、LLM、IO、网络等所有错误域。
//!
//! ## 设计要点
//!
//! - 基于 `thiserror` 派生 `Error`，自动实现 `Display` 和 `Error` trait
//! - 每个变体携带足够的上下文信息，便于日志记录和错误追踪
//! - 通过 `From` 实现自动转换，减少调用方的 `map_err` 样板代码

use std::env::VarError;

use thiserror::Error;

/// 项目级统一错误类型
#[derive(Debug, Error)]
pub enum AstrError {
    // ========== 会话相关 ==========
    #[error("session not found: {0}")]
    SessionNotFound(String),

    #[error("project not found: {0}")]
    ProjectNotFound(String),

    #[error("turn already in progress for session: {0}")]
    TurnInProgress(String),

    #[error("invalid session id: {0}")]
    InvalidSessionId(String),

    // ========== 配置相关 ==========
    #[error("profile '{profile}' {message}")]
    ConfigError { profile: String, message: String },

    #[error("missing api key for profile: {0}")]
    MissingApiKey(String),

    #[error("missing base url for profile: {0}")]
    MissingBaseUrl(String),

    #[error("no profiles configured")]
    NoProfilesConfigured,

    #[error("model '{model}' not found in profile '{profile}'")]
    ModelNotFound { profile: String, model: String },

    #[error("environment variable not found: {0}")]
    EnvVarNotFound(String),

    // ========== 工具相关 ==========
    #[error("tool '{name}' failed: {reason}")]
    ToolError { name: String, reason: String },

    // ========== LLM 相关 ==========
    #[error("LLM request failed: {status} - {body}")]
    LlmRequestFailed { status: u16, body: String },

    #[error("LLM stream error: {0}")]
    LlmStreamError(String),

    #[error("LLM request interrupted")]
    LlmInterrupted,

    #[error("invalid api key for provider: {0}")]
    InvalidApiKey(String),

    #[error("unsupported provider: {0}")]
    UnsupportedProvider(String),

    // ========== 操作状态 ==========
    #[error("operation cancelled")]
    Cancelled,

    #[error("lock poisoned: {0}")]
    LockPoisoned(String),

    // ========== IO/存储错误 ==========
    #[error("IO error: {context}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },

    #[error("parse error: {context}")]
    Parse {
        context: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("UTF-8 error: {context}")]
    Utf8 {
        context: String,
        #[source]
        source: std::str::Utf8Error,
    },

    // ========== 网络错误 ==========
    #[error("network error: {0}")]
    Network(String),

    #[error("HTTP request error: {context}")]
    HttpRequest {
        context: String,
        #[source]
        source: reqwest::Error,
    },

    // ========== 验证错误 ==========
    #[error("validation error: {0}")]
    Validation(String),

    // ========== 系统错误 ==========
    #[error("unsupported platform: {0}")]
    UnsupportedPlatform(String),

    #[error("home directory not found")]
    HomeDirectoryNotFound,

    #[error("internal error: {0}")]
    Internal(String),
}

// ========== From 实现 ==========

impl From<std::io::Error> for AstrError {
    fn from(e: std::io::Error) -> Self {
        AstrError::Io {
            context: String::new(),
            source: e,
        }
    }
}

impl From<serde_json::Error> for AstrError {
    fn from(e: serde_json::Error) -> Self {
        AstrError::Parse {
            context: String::new(),
            source: e,
        }
    }
}

impl From<std::str::Utf8Error> for AstrError {
    fn from(e: std::str::Utf8Error) -> Self {
        AstrError::Utf8 {
            context: String::new(),
            source: e,
        }
    }
}

impl From<VarError> for AstrError {
    fn from(e: VarError) -> Self {
        match e {
            VarError::NotPresent => AstrError::EnvVarNotFound(String::new()),
            VarError::NotUnicode(s) => AstrError::Internal(format!(
                "environment variable contains non-unicode data: {:?}",
                s
            )),
        }
    }
}

impl From<reqwest::Error> for AstrError {
    fn from(e: reqwest::Error) -> Self {
        AstrError::HttpRequest {
            context: String::new(),
            source: e,
        }
    }
}

// ========== 辅助方法 ==========

impl AstrError {
    /// 为 IO/Parse/Utf8/HttpRequest 错误添加上下文信息
    pub fn context(self, context: impl Into<String>) -> Self {
        let context = context.into();
        match self {
            AstrError::Io { source, .. } => AstrError::Io { context, source },
            AstrError::Parse { source, .. } => AstrError::Parse { context, source },
            AstrError::Utf8 { source, .. } => AstrError::Utf8 { context, source },
            AstrError::HttpRequest { source, .. } => AstrError::HttpRequest { context, source },
            other => other,
        }
    }

    pub fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        AstrError::Io {
            context: context.into(),
            source,
        }
    }

    pub fn parse(context: impl Into<String>, source: serde_json::Error) -> Self {
        AstrError::Parse {
            context: context.into(),
            source,
        }
    }

    pub fn http(context: impl Into<String>, source: reqwest::Error) -> Self {
        AstrError::HttpRequest {
            context: context.into(),
            source,
        }
    }

    /// 检查是否为可重试的网络错误
    pub fn is_retryable(&self) -> bool {
        match self {
            AstrError::HttpRequest { source, .. } => {
                source.is_timeout() || source.is_connect() || source.is_body()
            },
            _ => false,
        }
    }

    /// 检查是否为取消错误
    pub fn is_cancelled(&self) -> bool {
        matches!(self, AstrError::Cancelled | AstrError::LlmInterrupted)
    }

    /// 检查是否为上下文窗口超限错误。
    ///
    /// 为什么不再依赖 `Display` 文本：
    /// prompt-too-long 的恢复决策只应依赖 LLM 错误负载本身，
    /// 不应被外围包装前缀或日志措辞变化影响。
    pub fn is_prompt_too_long(&self) -> bool {
        match self {
            AstrError::LlmRequestFailed { status, body } => {
                matches!(*status, 400 | 413) && is_prompt_too_long_message(body)
            },
            AstrError::LlmStreamError(message)
            | AstrError::Validation(message)
            | AstrError::Internal(message) => is_prompt_too_long_message(message),
            _ => false,
        }
    }
}

fn is_prompt_too_long_message(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("prompt too long")
        || lower.contains("context length")
        || lower.contains("maximum context")
        || lower.contains("too many tokens")
}

/// 用于链式添加错误上下文的 trait
pub trait ResultExt<T> {
    fn context(self, context: impl Into<String>) -> Result<T>;
    fn with_context<F: FnOnce() -> String>(self, f: F) -> Result<T>;
}

impl<T> ResultExt<T> for Result<T> {
    fn context(self, context: impl Into<String>) -> Result<T> {
        self.map_err(|e| e.context(context))
    }

    fn with_context<F: FnOnce() -> String>(self, f: F) -> Result<T> {
        self.map_err(|e| e.context(f()))
    }
}

/// 项目级 Result 类型
pub type Result<T> = std::result::Result<T, AstrError>;
