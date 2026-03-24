use thiserror::Error;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ErrorPayload {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub details: Value,
    #[serde(default)]
    pub retriable: bool,
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("unsupported protocol version: {0}")]
    UnsupportedVersion(String),
    #[error("invalid message: {0}")]
    InvalidMessage(String),
    #[error("request cancelled: {0}")]
    Cancelled(String),
    #[error("transport closed: {0}")]
    TransportClosed(String),
    #[error("unexpected message: {0}")]
    UnexpectedMessage(String),
}
