use astrcode_protocol::plugin::ErrorPayload;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSerdeStage {
    DecodeInput,
    EncodeOutput,
}

impl ToolSerdeStage {
    pub fn action(self) -> &'static str {
        match self {
            Self::DecodeInput => "decode input",
            Self::EncodeOutput => "encode output",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Error)]
pub enum SdkError {
    #[error(
        "tool '{capability}' failed to {action} as {rust_type}: {message}",
        action = .stage.action()
    )]
    Serde {
        capability: String,
        stage: ToolSerdeStage,
        rust_type: &'static str,
        message: String,
    },
    #[error("validation failed: {message}")]
    Validation { message: String, details: Value },
    #[error("permission denied: {message}")]
    PermissionDenied { message: String, details: Value },
    #[error("request cancelled")]
    Cancelled,
    #[error("i/o error: {message}")]
    Io { message: String },
    #[error("stream emission failed for event '{event}': {message}")]
    StreamEmit {
        event: String,
        message: String,
        details: Value,
    },
    #[error("internal error: {message}")]
    Internal {
        message: String,
        details: Value,
        retriable: bool,
    },
}

impl SdkError {
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

    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
            details: Value::Null,
        }
    }

    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self::PermissionDenied {
            message: message.into(),
            details: Value::Null,
        }
    }

    pub fn cancelled() -> Self {
        Self::Cancelled
    }

    pub fn io(error: std::io::Error) -> Self {
        Self::Io {
            message: error.to_string(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
            details: Value::Null,
            retriable: false,
        }
    }

    pub fn stream_emit(event: impl Into<String>, message: impl Into<String>) -> Self {
        Self::StreamEmit {
            event: event.into(),
            message: message.into(),
            details: Value::Null,
        }
    }

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

    pub fn retriable(&self) -> bool {
        match self {
            Self::Internal { retriable, .. } => *retriable,
            _ => false,
        }
    }

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
        Self::Validation {
            message: value.to_string(),
            details: Value::Null,
        }
    }
}

impl From<String> for SdkError {
    fn from(value: String) -> Self {
        Self::Validation {
            message: value,
            details: Value::Null,
        }
    }
}

impl From<&str> for SdkError {
    fn from(value: &str) -> Self {
        Self::Validation {
            message: value.to_string(),
            details: Value::Null,
        }
    }
}
