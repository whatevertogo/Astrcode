use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::SdkError;

type StreamResult<T> = Result<T, SdkError>;
type StreamCallback = dyn Fn(StreamChunk) -> StreamResult<()> + Send + Sync;

#[derive(Debug, Clone, PartialEq)]
pub struct StreamChunk {
    pub event: String,
    pub payload: Value,
}

#[derive(Clone, Default)]
pub struct StreamWriter {
    records: Arc<Mutex<Vec<StreamChunk>>>,
    callback: Option<Arc<StreamCallback>>,
}

impl StreamWriter {
    pub fn with_callback<F>(callback: F) -> Self
    where
        F: Fn(StreamChunk) -> StreamResult<()> + Send + Sync + 'static,
    {
        Self {
            records: Arc::new(Mutex::new(Vec::new())),
            callback: Some(Arc::new(callback)),
        }
    }

    pub fn emit(&self, event: impl Into<String>, payload: Value) -> StreamResult<()> {
        let event = event.into();
        let chunk = StreamChunk {
            event: event.clone(),
            payload,
        };
        self.records
            .lock()
            .map_err(|_| SdkError::internal("stream records lock poisoned"))?
            .push(chunk.clone());
        if let Some(callback) = &self.callback {
            callback(chunk).map_err(|error| SdkError::StreamEmit {
                event,
                message: error.to_string(),
                details: error.details(),
            })?;
        }
        Ok(())
    }

    pub fn message_delta(&self, text: impl Into<String>) -> StreamResult<()> {
        self.emit("message.delta", json!({ "text": text.into() }))
    }

    pub fn artifact_patch(
        &self,
        path: impl Into<String>,
        patch: impl Into<String>,
    ) -> StreamResult<()> {
        self.emit(
            "artifact.patch",
            json!({
                "path": path.into(),
                "patch": patch.into(),
            }),
        )
    }

    pub fn diagnostic(
        &self,
        severity: impl Into<String>,
        message: impl Into<String>,
    ) -> StreamResult<()> {
        self.emit(
            "diagnostic",
            json!({
                "severity": severity.into(),
                "message": message.into(),
            }),
        )
    }

    pub fn records(&self) -> Vec<StreamChunk> {
        self.records.lock().expect("stream records lock").clone()
    }
}
