use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ErrorPayload, InitializeMessage, InvocationContext, ProtocolError};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InvokeMessage {
    pub id: String,
    pub capability: String,
    pub input: Value,
    pub context: InvocationContext,
    #[serde(default)]
    pub stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventPhase {
    Started,
    Delta,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EventMessage {
    pub id: String,
    pub phase: EventPhase,
    pub event: String,
    #[serde(default)]
    pub payload: Value,
    pub seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CancelMessage {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ResultMessage {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub success: bool,
    #[serde(default)]
    pub output: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorPayload>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PluginMessage {
    Initialize(InitializeMessage),
    Invoke(InvokeMessage),
    Result(ResultMessage),
    Event(EventMessage),
    Cancel(CancelMessage),
}

impl ResultMessage {
    pub fn success(id: impl Into<String>, output: Value) -> Self {
        Self {
            id: id.into(),
            kind: None,
            success: true,
            output,
            error: None,
            metadata: Value::Null,
        }
    }

    pub fn failure(id: impl Into<String>, error: ErrorPayload) -> Self {
        Self {
            id: id.into(),
            kind: None,
            success: false,
            output: Value::Null,
            error: Some(error),
            metadata: Value::Null,
        }
    }

    pub fn parse_output<T>(&self) -> Result<T, ProtocolError>
    where
        T: serde::de::DeserializeOwned,
    {
        serde_json::from_value(self.output.clone())
            .map_err(|error| ProtocolError::InvalidMessage(error.to_string()))
    }
}
