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

    // ========== 工具相关 ==========
    #[error("tool '{name}' failed: {reason}")]
    ToolError { name: String, reason: String },

    #[error("path escapes sandbox: {path}")]
    SandboxEscape { path: String },

    // ========== LLM 相关 ==========
    #[error("LLM request failed: {status} - {body}")]
    LlmRequestFailed { status: u16, body: String },

    #[error("LLM stream error: {0}")]
    LlmStreamError(String),

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

// ========== 辅助方法 ==========

impl AstrError {
    pub fn with_context(self, context: impl Into<String>) -> Self {
        match self {
            AstrError::Io { source, .. } => AstrError::Io {
                context: context.into(),
                source,
            },
            AstrError::Parse { source, .. } => AstrError::Parse {
                context: context.into(),
                source,
            },
            AstrError::Utf8 { source, .. } => AstrError::Utf8 {
                context: context.into(),
                source,
            },
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
}

/// 项目级 Result 类型
pub type Result<T> = std::result::Result<T, AstrError>;
