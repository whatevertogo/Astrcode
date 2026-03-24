use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use astrcode_core::Result;
use astrcode_protocol::plugin::EventMessage;
use serde_json::Value;
use tokio::sync::mpsc;

type EmitFuture = Pin<Box<dyn Future<Output = Result<()>> + Send>>;
type EmitFn = dyn Fn(String, Value) -> EmitFuture + Send + Sync;

#[derive(Clone, Default)]
pub struct EventEmitter {
    emit: Option<Arc<EmitFn>>,
}

impl EventEmitter {
    pub fn new<F, Fut>(emit: F) -> Self
    where
        F: Fn(String, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        Self {
            emit: Some(Arc::new(move |event, payload| {
                Box::pin(emit(event, payload))
            })),
        }
    }

    pub fn noop() -> Self {
        Self { emit: None }
    }

    pub async fn delta(&self, event: impl Into<String>, payload: Value) -> Result<()> {
        match &self.emit {
            Some(emit) => emit(event.into(), payload).await,
            None => Ok(()),
        }
    }
}

pub struct StreamExecution {
    request_id: String,
    receiver: mpsc::UnboundedReceiver<EventMessage>,
}

impl StreamExecution {
    pub fn new(
        request_id: impl Into<String>,
        receiver: mpsc::UnboundedReceiver<EventMessage>,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            receiver,
        }
    }

    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub async fn recv(&mut self) -> Option<EventMessage> {
        self.receiver.recv().await
    }
}
