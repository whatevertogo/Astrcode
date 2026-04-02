//! SDK 错误类型体系。
//!
//! 本模块定义了插件 SDK 的统一错误类型 `SdkError`，
//! 覆盖工具执行生命周期中可能出现的所有错误场景。
//!
//! ## 设计原则
//!
//! - **分类明确**: 每种错误变体对应一种失败模式，调用方可据此决定重试或降级策略
//! - **可序列化**: 所有错误都能转换为 `ErrorPayload` 发送给前端或协议层
//! - **便捷构造**: 提供语义化的构造函数（如 `SdkError::validation()`），避免手动拼凑变体
//!
//! ## 错误分类
//!
//! | 变体 | 触发场景 | 可重试 |
//! |------|---------|--------|
//! | `Serde` | 输入解码或输出编码失败 | 否 |
//! | `Validation` | 工具输入校验不通过 | 否 |
//! | `PermissionDenied` | 策略钩子拒绝执行 | 否 |
//! | `Cancelled` | 请求被取消 | 否 |
//! | `Io` | 文件系统或网络 I/O 失败 | 视情况 |
//! | `StreamEmit` | 流式事件发送失败 | 视情况 |
//! | `Internal` | 插件内部未预期错误 | 由 `retriable` 标记 |

use astrcode_protocol::plugin::ErrorPayload;
use serde_json::Value;
use thiserror::Error;

/// 标识序列化/反序列化失败发生在工具执行的哪个阶段。
///
/// 用于错误消息中准确描述是"解码输入"还是"编码输出"出了问题，
/// 帮助插件作者快速定位问题方向。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSerdeStage {
    /// 将 JSON 输入反序列化为工具期望的 Rust 类型时失败。
    DecodeInput,
    /// 将工具返回的 Rust 类型序列化为 JSON 输出时失败。
    EncodeOutput,
}

impl ToolSerdeStage {
    /// 返回该阶段的人类可读描述，用于错误消息拼接。
    pub fn action(self) -> &'static str {
        match self {
            Self::DecodeInput => "decode input",
            Self::EncodeOutput => "encode output",
        }
    }
}

/// SDK 统一错误类型。
///
/// 覆盖插件工具执行生命周期中的所有错误场景，
/// 每个变体都携带足够的上下文信息用于调试和前端展示。
///
/// ## 错误码映射
///
/// 通过 `code()` 方法可获取机器可读的错误码，
/// 用于前端根据错误类型采取不同的 UI 展示策略。
#[derive(Debug, Clone, PartialEq, Error)]
pub enum SdkError {
    /// 序列化/反序列化失败。
    ///
    /// 发生在工具输入解码或输出编码阶段，
    /// 通常意味着插件定义的输入/输出类型与实际传输的 JSON 不匹配。
    #[error(
        "tool '{capability}' failed to {action} as {rust_type}: {message}",
        action = .stage.action()
    )]
    Serde {
        /// 工具/能力名称。
        capability: String,
        /// 失败发生的阶段。
        stage: ToolSerdeStage,
        /// 涉及的 Rust 类型名称。
        rust_type: &'static str,
        /// 底层 serde_json 的错误消息。
        message: String,
    },
    /// 输入校验失败。
    ///
    /// 工具在执行业务逻辑前发现输入不符合预期，
    /// 例如必填字段缺失、值超出合法范围等。
    #[error("validation failed: {message}")]
    Validation {
        /// 人类可读的错误描述。
        message: String,
        /// 结构化的错误详情，可包含字段级错误等信息。
        details: Value,
    },
    /// 权限被策略钩子拒绝。
    ///
    /// 插件注册的 `PolicyHook` 在工具执行前返回了 deny 决策。
    #[error("permission denied: {message}")]
    PermissionDenied {
        /// 拒绝原因。
        message: String,
        /// 策略钩子附加的额外信息。
        details: Value,
    },
    /// 请求被取消。
    ///
    /// 用户主动取消或超时导致工具执行被中止。
    #[error("request cancelled")]
    Cancelled,
    /// I/O 操作失败。
    ///
    /// 文件读写、网络请求等底层 I/O 错误。
    #[error("i/o error: {message}")]
    Io {
        /// 底层 I/O 错误的描述。
        message: String,
    },
    /// 流式事件发送失败。
    ///
    /// 通过 `StreamWriter` 发送增量输出时发生错误，
    /// 可能是回调函数返回错误或内部状态异常。
    #[error("stream emission failed for event '{event}': {message}")]
    StreamEmit {
        /// 事件名称。
        event: String,
        /// 错误描述。
        message: String,
        /// 附加的错误上下文。
        details: Value,
    },
    /// 插件内部未预期的错误。
    ///
    /// 不属于以上分类的兜底错误，通常表示代码中的 bug 或
    /// 未处理的边界情况。可通过 `retriable` 标记建议是否重试。
    #[error("internal error: {message}")]
    Internal {
        /// 错误描述。
        message: String,
        /// 结构化的错误详情。
        details: Value,
        /// 是否建议重试。
        retriable: bool,
    },
}

impl SdkError {
    /// 构造输入解码失败的错误。
    ///
    /// 当工具无法将传入的 JSON 解析为期望的 Rust 类型时调用。
    pub fn decode_input(
        capability: impl Into<String>,
        rust_type: &'static str,
        source: serde_json::Error,
    ) -> Self {
        Self::Serde {
            capability: capability.into(),
            stage: ToolSerdeStage::DecodeInput,
            rust_type,
            message: source.to_string(),
        }
    }

    /// 构造输出编码失败的错误。
    ///
    /// 当工具返回的 Rust 值无法序列化为 JSON 时调用。
    pub fn encode_output(
        capability: impl Into<String>,
        rust_type: &'static str,
        source: serde_json::Error,
    ) -> Self {
        Self::Serde {
            capability: capability.into(),
            stage: ToolSerdeStage::EncodeOutput,
            rust_type,
            message: source.to_string(),
        }
    }

    /// 构造输入校验失败的错误。
    ///
    /// 用于工具在业务逻辑执行前发现输入不合法的场景，
    /// 例如参数超出范围、必填字段缺失等。
    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
            details: Value::Null,
        }
    }

    /// 构造权限拒绝的错误。
    ///
    /// 通常由策略钩子调用，表示当前操作不被允许。
    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self::PermissionDenied {
            message: message.into(),
            details: Value::Null,
        }
    }

    /// 构造请求取消的错误。
    ///
    /// 表示用户主动取消或超时导致工具执行被中止。
    pub fn cancelled() -> Self {
        Self::Cancelled
    }

    /// 从 `std::io::Error` 构造 I/O 错误。
    pub fn io(error: std::io::Error) -> Self {
        Self::Io {
            message: error.to_string(),
        }
    }

    /// 构造内部错误。
    ///
    /// 用于不属于其他分类的兜底错误，通常表示未预期的异常。
    /// 默认 `retriable` 为 `false`，如需标记为可重试请使用 `details` 变体。
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
            details: Value::Null,
            retriable: false,
        }
    }

    /// 构造流式事件发送失败的错误。
    ///
    /// 当通过 `StreamWriter` 发送增量输出失败时调用。
    pub fn stream_emit(event: impl Into<String>, message: impl Into<String>) -> Self {
        Self::StreamEmit {
            event: event.into(),
            message: message.into(),
            details: Value::Null,
        }
    }

    /// 返回机器可读的错误码。
    ///
    /// 用于前端或协议层根据错误类型采取不同的处理策略，
    /// 例如 `validation_error` 显示表单错误，`io_error` 提示重试等。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Serde { stage, .. } => match stage {
                ToolSerdeStage::DecodeInput => "invalid_input",
                ToolSerdeStage::EncodeOutput => "invalid_output",
            },
            Self::Validation { .. } => "validation_error",
            Self::PermissionDenied { .. } => "permission_denied",
            Self::Cancelled => "cancelled",
            Self::Io { .. } => "io_error",
            Self::StreamEmit { .. } => "stream_error",
            Self::Internal { .. } => "internal_error",
        }
    }

    /// 返回该错误是否建议重试。
    ///
    /// 仅 `Internal` 变体可标记为可重试，其他错误类型默认不可重试，
    /// 因为重试相同输入大概率会得到相同结果。
    pub fn retriable(&self) -> bool {
        match self {
            Self::Internal { retriable, .. } => *retriable,
            _ => false,
        }
    }

    /// 返回结构化的错误详情。
    ///
    /// 对于 `Serde` 错误，详情包含 capability、stage、rustType 等诊断信息；
    /// 对于其他变体，返回构造时传入的 `details` 字段。
    pub fn details(&self) -> Value {
        match self {
            Self::Serde {
                capability,
                stage,
                rust_type,
                message,
            } => Value::Object(serde_json::Map::from_iter([
                ("capability".to_string(), Value::String(capability.clone())),
                (
                    "stage".to_string(),
                    Value::String(
                        match stage {
                            ToolSerdeStage::DecodeInput => "decode_input",
                            ToolSerdeStage::EncodeOutput => "encode_output",
                        }
                        .to_string(),
                    ),
                ),
                (
                    "rustType".to_string(),
                    Value::String((*rust_type).to_string()),
                ),
                ("message".to_string(), Value::String(message.clone())),
            ])),
            Self::Validation { details, .. }
            | Self::PermissionDenied { details, .. }
            | Self::StreamEmit { details, .. }
            | Self::Internal { details, .. } => details.clone(),
            Self::Cancelled | Self::Io { .. } => Value::Null,
        }
    }

    /// 将错误转换为协议层的 `ErrorPayload`。
    ///
    /// 用于将 SDK 内部错误序列化为可传输的格式，
    /// 发送给前端或写入事件日志。
    pub fn to_error_payload(&self) -> ErrorPayload {
        ErrorPayload {
            code: self.code().to_string(),
            message: self.to_string(),
            details: self.details(),
            retriable: self.retriable(),
        }
    }
}

impl From<std::io::Error> for SdkError {
    fn from(value: std::io::Error) -> Self {
        Self::io(value)
    }
}

impl From<serde_json::Error> for SdkError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serde {
            capability: "unknown".to_string(),
            stage: ToolSerdeStage::DecodeInput,
            rust_type: "unknown",
            message: value.to_string(),
        }
    }
}

impl From<String> for SdkError {
    fn from(value: String) -> Self {
        Self::internal(value)
    }
}

impl From<&str> for SdkError {
    fn from(value: &str) -> Self {
        Self::internal(value)
    }
}
