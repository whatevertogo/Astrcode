use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("unsupported protocol version: {0}")]
    UnsupportedVersion(String),
    #[error("invalid message: {0}")]
    InvalidMessage(String),
    #[error("request cancelled: {0}")]
    Cancelled(String),
}
