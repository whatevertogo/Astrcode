//! Astrcode 离线评测框架。

pub mod diagnosis;
pub mod runner;
pub mod task;
pub mod trace;

use std::path::PathBuf;

use thiserror::Error;

/// 评测 crate 统一错误类型。
#[derive(Debug, Error)]
pub enum EvalError {
    #[error("IO 错误: {message}")]
    Io {
        message: String,
        #[source]
        source: std::io::Error,
    },
    #[error("JSON 解析失败（第 {line} 行）: {source}")]
    JsonLine {
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("YAML 解析失败（{path}）: {source}")]
    Yaml {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },
    #[error("HTTP 请求失败: {message}")]
    Http {
        message: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("{0}")]
    Timeout(String),
    #[error("glob 模式无效（{pattern}）: {source}")]
    GlobPattern {
        pattern: String,
        #[source]
        source: glob::PatternError,
    },
    #[error("glob 遍历失败（{pattern}）: {source}")]
    GlobWalk {
        pattern: String,
        #[source]
        source: glob::GlobError,
    },
    #[error("{0}")]
    Validation(String),
}

impl EvalError {
    pub fn io(message: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            message: message.into(),
            source,
        }
    }

    pub fn yaml(path: impl Into<String>, source: serde_yaml::Error) -> Self {
        Self::Yaml {
            path: path.into(),
            source,
        }
    }

    pub fn http(message: impl Into<String>, source: reqwest::Error) -> Self {
        Self::Http {
            message: message.into(),
            source,
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation(message.into())
    }

    pub fn timeout(message: impl Into<String>) -> Self {
        Self::Timeout(message.into())
    }
}

pub type EvalResult<T> = Result<T, EvalError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskLoadWarning {
    pub path: PathBuf,
    pub message: String,
}
