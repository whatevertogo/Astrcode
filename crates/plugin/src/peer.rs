use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use astrcode_core::{AstrError, CancelToken, Result};
use astrcode_protocol::plugin::{
    CancelMessage, ErrorPayload, EventMessage, EventPhase, InitializeMessage, InitializeResultData,
    InvokeMessage, PluginMessage, ResultMessage, PROTOCOL_VERSION,
};
use astrcode_protocol::transport::Transport;
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot, Mutex, Notify};

use crate::{CapabilityRouter, EventEmitter, StreamExecution};

#[derive(Clone)]
pub struct Peer {
    inner: Arc<PeerInner>,
}

struct PeerInner {
    transport: Arc<dyn Transport>,
    local_initialize: InitializeMessage,
    router: Arc<CapabilityRouter>,
    pending_results: Mutex<HashMap<String, oneshot::Sender<ResultMessage>>>,
    pending_streams: Mutex<HashMap<String, mpsc::UnboundedSender<EventMessage>>>,
    inbound_cancellations: Mutex<HashMap<String, CancelToken>>,
    remote_initialize: Mutex<Option<InitializeResultData>>,
    closed_reason: Mutex<Option<String>>,
    closed_notify: Notify,
}

impl Peer {
    pub fn new(
        transport: Arc<dyn Transport>,
        local_initialize: InitializeMessage,
        router: Arc<CapabilityRouter>,
    ) -> Self {
        let inner = Arc::new(PeerInner {
            transport,
            local_initialize,
            router,
            pending_results: Mutex::new(HashMap::new()),
            pending_streams: Mutex::new(HashMap::new()),
            inbound_cancellations: Mutex::new(HashMap::new()),
            remote_initialize: Mutex::new(None),
            closed_reason: Mutex::new(None),
            closed_notify: Notify::new(),
        });

        let peer = Self { inner };
        peer.spawn_read_loop();
        peer
    }

    pub async fn initialize(&self) -> Result<InitializeResultData> {
        let request = self.inner.local_initialize.clone();
        let response = self.await_result(request).await?;
        if response.kind.as_deref() != Some("initialize") {
            return Err(AstrError::Internal(format!(
                "expected initialize result for '{}', received kind {:?}",
                response.id, response.kind
            )));
        }
        if !response.success {
            return Err(result_error_to_astr(response));
        }
        let negotiated: InitializeResultData = response.parse_output().map_err(|error| {
            AstrError::Validation(format!("failed to parse initialize result: {error}"))
        })?;
        *self.inner.remote_initialize.lock().await = Some(negotiated.clone());
        Ok(negotiated)
    }

    pub async fn invoke(&self, request: InvokeMessage) -> Result<ResultMessage> {
        self.await_result(request).await
    }

    pub async fn invoke_stream(&self, request: InvokeMessage) -> Result<StreamExecution> {
        let request_id = request.id.clone();
        let (sender, receiver) = mpsc::unbounded_channel();
        self.inner
            .pending_streams
            .lock()
            .await
            .insert(request_id.clone(), sender);
        let send_result = self.send_message(&PluginMessage::Invoke(request)).await;
        if let Err(error) = send_result {
            self.inner.pending_streams.lock().await.remove(&request_id);
            return Err(error);
        }
        Ok(StreamExecution::new(request_id, receiver))
    }

    pub async fn cancel(
        &self,
        request_id: impl Into<String>,
        reason: Option<String>,
    ) -> Result<()> {
        self.send_message(&PluginMessage::Cancel(CancelMessage {
            id: request_id.into(),
            reason,
        }))
        .await
    }

    pub async fn remote_initialize(&self) -> Option<InitializeResultData> {
        self.inner.remote_initialize.lock().await.clone()
    }

    pub async fn closed_reason(&self) -> Option<String> {
        self.inner.closed_reason.lock().await.clone()
    }

    pub async fn wait_closed(&self) {
        loop {
            let notified = self.inner.closed_notify.notified();
            if self.inner.closed_reason.lock().await.is_some() {
                return;
            }
            notified.await;
        }
    }

    fn spawn_read_loop(&self) {
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            inner.read_loop().await;
        });
    }

    async fn await_result<T>(&self, request: T) -> Result<ResultMessage>
    where
        T: Into<InvokeOrInitialize>,
    {
        let request = request.into();
        let request_id = request.id().to_string();
        let (sender, receiver) = oneshot::channel();
        self.inner
            .pending_results
            .lock()
            .await
            .insert(request_id.clone(), sender);
        if let Err(error) = self.send_message(&request.into_message()).await {
            self.inner.pending_results.lock().await.remove(&request_id);
            return Err(error);
        }
        receiver.await.map_err(|_| {
            AstrError::Internal(format!(
                "peer dropped pending result channel '{}'",
                request_id
            ))
        })
    }

    async fn send_message(&self, message: &PluginMessage) -> Result<()> {
        self.inner.send_message(message).await
    }
}

enum InvokeOrInitialize {
    Initialize(InitializeMessage),
    Invoke(InvokeMessage),
}

impl InvokeOrInitialize {
    fn id(&self) -> &str {
        match self {
            Self::Initialize(message) => &message.id,
            Self::Invoke(message) => &message.id,
        }
    }

    fn into_message(self) -> PluginMessage {
        match self {
            Self::Initialize(message) => PluginMessage::Initialize(message),
            Self::Invoke(message) => PluginMessage::Invoke(message),
        }
    }
}

impl From<InitializeMessage> for InvokeOrInitialize {
    fn from(value: InitializeMessage) -> Self {
        Self::Initialize(value)
    }
}

impl From<InvokeMessage> for InvokeOrInitialize {
    fn from(value: InvokeMessage) -> Self {
        Self::Invoke(value)
    }
}

impl PeerInner {
    async fn read_loop(self: Arc<Self>) {
        loop {
            match self.transport.recv().await {
                Ok(Some(payload)) => {
                    let message = match serde_json::from_str::<PluginMessage>(&payload) {
                        Ok(message) => message,
                        Err(error) => {
                            self.close(format!("failed to decode plugin message: {error}"))
                                .await;
                            break;
                        }
                    };
                    if let Err(error) = Arc::clone(&self).handle_message(message).await {
                        self.close(format!("peer message handling failed: {error}"))
                            .await;
                        break;
                    }
                }
                Ok(None) => {
                    self.close("transport closed".to_string()).await;
                    break;
                }
                Err(error) => {
                    self.close(error).await;
                    break;
                }
            }
        }
    }

    async fn handle_message(self: Arc<Self>, message: PluginMessage) -> Result<()> {
        match message {
            PluginMessage::Initialize(message) => self.handle_initialize(message).await,
            PluginMessage::Invoke(message) => {
                self.handle_invoke(message);
                Ok(())
            }
            PluginMessage::Result(message) => self.handle_result(message).await,
            PluginMessage::Event(message) => self.handle_event(message).await,
            PluginMessage::Cancel(message) => self.handle_cancel(message).await,
        }
    }

    async fn handle_initialize(&self, message: InitializeMessage) -> Result<()> {
        let InitializeMessage {
            id,
            protocol_version,
            supported_protocol_versions,
            peer,
            capabilities,
            handlers,
            profiles,
            metadata,
        } = message;
        let supported = protocol_version == PROTOCOL_VERSION
            || supported_protocol_versions
                .iter()
                .any(|version| version == PROTOCOL_VERSION);
        let response = if supported {
            let negotiated = InitializeResultData {
                protocol_version: PROTOCOL_VERSION.to_string(),
                peer,
                capabilities,
                handlers,
                profiles,
                metadata,
            };
            *self.remote_initialize.lock().await = Some(negotiated.clone());
            ResultMessage {
                id,
                kind: Some("initialize".to_string()),
                success: true,
                output: serde_json::to_value(self.local_result()).map_err(|error| {
                    AstrError::Validation(format!(
                        "failed to serialize local initialize result: {error}"
                    ))
                })?,
                error: None,
                metadata: json!({ "acceptedVersion": PROTOCOL_VERSION }),
            }
        } else {
            ResultMessage {
                id,
                kind: Some("initialize".to_string()),
                success: false,
                output: Value::Null,
                error: Some(ErrorPayload {
                    code: "unsupported_version".to_string(),
                    message: format!(
                        "peer version '{}' does not support '{}'",
                        protocol_version, PROTOCOL_VERSION
                    ),
                    details: json!({ "supportedProtocolVersions": supported_protocol_versions }),
                    retriable: false,
                }),
                metadata: Value::Null,
            }
        };
        self.send_message(&PluginMessage::Result(response)).await
    }

    fn handle_invoke(self: Arc<Self>, message: InvokeMessage) {
        tokio::spawn(async move {
            let request_id = message.id.clone();
            let cancel = CancelToken::new();
            self.inbound_cancellations
                .lock()
                .await
                .insert(request_id.clone(), cancel.clone());

            let result = if message.stream {
                self.handle_streaming_invoke(message, cancel.clone()).await
            } else {
                self.handle_unary_invoke(message, cancel.clone()).await
            };

            self.inbound_cancellations.lock().await.remove(&request_id);
            if let Err(error) = result {
                self.close(format!("failed to process inbound invoke: {error}"))
                    .await;
            }
        });
    }

    async fn handle_unary_invoke(&self, message: InvokeMessage, cancel: CancelToken) -> Result<()> {
        let result = self
            .router
            .invoke(
                &message.capability,
                message.input,
                message.context,
                EventEmitter::noop(),
                cancel,
            )
            .await;
        let response = match result {
            Ok(output) => ResultMessage::success(message.id, output),
            Err(error) => ResultMessage::failure(message.id, error_to_payload(&error)),
        };
        self.send_message(&PluginMessage::Result(response)).await
    }

    async fn handle_streaming_invoke(
        self: &Arc<Self>,
        message: InvokeMessage,
        cancel: CancelToken,
    ) -> Result<()> {
        let request_id = message.id.clone();
        let sequence = Arc::new(AtomicU64::new(1));
        self.send_message(&PluginMessage::Event(EventMessage {
            id: request_id.clone(),
            phase: EventPhase::Started,
            event: "invoke.started".to_string(),
            payload: json!({ "capability": message.capability }),
            seq: 0,
            error: None,
        }))
        .await?;

        let transport = Arc::clone(&self.transport);
        let emit_request_id = request_id.clone();
        let emit_sequence = Arc::clone(&sequence);
        let emitter = EventEmitter::new(move |event, payload| {
            let transport = Arc::clone(&transport);
            let emit_request_id = emit_request_id.clone();
            let emit_sequence = Arc::clone(&emit_sequence);
            async move {
                let event_message = PluginMessage::Event(EventMessage {
                    id: emit_request_id,
                    phase: EventPhase::Delta,
                    event,
                    payload,
                    seq: emit_sequence.fetch_add(1, Ordering::SeqCst),
                    error: None,
                });
                send_message_via_transport(transport, &event_message).await
            }
        });

        let result = self
            .router
            .invoke(
                &message.capability,
                message.input,
                message.context,
                emitter,
                cancel.clone(),
            )
            .await;

        let terminal = match result {
            Ok(output) if cancel.is_cancelled() => EventMessage {
                id: request_id,
                phase: EventPhase::Failed,
                event: "invoke.cancelled".to_string(),
                payload: Value::Null,
                seq: sequence.fetch_add(1, Ordering::SeqCst),
                error: Some(ErrorPayload {
                    code: "cancelled".to_string(),
                    message: "request was cancelled".to_string(),
                    details: Value::Null,
                    retriable: false,
                }),
            },
            Ok(output) => EventMessage {
                id: request_id,
                phase: EventPhase::Completed,
                event: "invoke.completed".to_string(),
                payload: output,
                seq: sequence.fetch_add(1, Ordering::SeqCst),
                error: None,
            },
            Err(error) => EventMessage {
                id: request_id,
                phase: EventPhase::Failed,
                event: "invoke.failed".to_string(),
                payload: Value::Null,
                seq: sequence.fetch_add(1, Ordering::SeqCst),
                error: Some(error_to_payload(&error)),
            },
        };

        self.send_message(&PluginMessage::Event(terminal)).await
    }

    async fn handle_result(&self, message: ResultMessage) -> Result<()> {
        if let Some(sender) = self.pending_results.lock().await.remove(&message.id) {
            let _ = sender.send(message);
        }
        Ok(())
    }

    async fn handle_event(&self, message: EventMessage) -> Result<()> {
        let is_terminal = matches!(message.phase, EventPhase::Completed | EventPhase::Failed);
        let mut streams = self.pending_streams.lock().await;
        if let Some(sender) = streams.get(&message.id) {
            let _ = sender.send(message.clone());
        }
        if is_terminal {
            streams.remove(&message.id);
        }
        Ok(())
    }

    async fn handle_cancel(&self, message: CancelMessage) -> Result<()> {
        if let Some(cancel) = self
            .inbound_cancellations
            .lock()
            .await
            .get(&message.id)
            .cloned()
        {
            cancel.cancel();
        }
        Ok(())
    }

    fn local_result(&self) -> InitializeResultData {
        InitializeResultData {
            protocol_version: self.local_initialize.protocol_version.clone(),
            peer: self.local_initialize.peer.clone(),
            capabilities: self.local_initialize.capabilities.clone(),
            handlers: self.local_initialize.handlers.clone(),
            profiles: self.local_initialize.profiles.clone(),
            metadata: self.local_initialize.metadata.clone(),
        }
    }

    async fn send_message(&self, message: &PluginMessage) -> Result<()> {
        send_message_via_transport(Arc::clone(&self.transport), message).await
    }

    async fn close(&self, reason: String) {
        let mut closed_reason = self.closed_reason.lock().await;
        if closed_reason.is_some() {
            return;
        }
        *closed_reason = Some(reason.clone());
        drop(closed_reason);

        let error = ErrorPayload {
            code: "transport_closed".to_string(),
            message: reason.clone(),
            details: Value::Null,
            retriable: false,
        };

        let pending_results = std::mem::take(&mut *self.pending_results.lock().await);
        for (request_id, sender) in pending_results {
            let _ = sender.send(ResultMessage::failure(request_id, error.clone()));
        }

        let pending_streams = std::mem::take(&mut *self.pending_streams.lock().await);
        for (request_id, sender) in pending_streams {
            let _ = sender.send(EventMessage {
                id: request_id,
                phase: EventPhase::Failed,
                event: "transport.closed".to_string(),
                payload: Value::Null,
                seq: 0,
                error: Some(error.clone()),
            });
        }

        self.closed_notify.notify_waiters();
    }
}

async fn send_message_via_transport(
    transport: Arc<dyn Transport>,
    message: &PluginMessage,
) -> Result<()> {
    let payload = serde_json::to_string(message).map_err(|error| {
        AstrError::Validation(format!("failed to serialize plugin message: {error}"))
    })?;
    transport
        .send(&payload)
        .await
        .map_err(|error| AstrError::Internal(format!("failed to send plugin message: {error}")))
}

fn error_to_payload(error: &AstrError) -> ErrorPayload {
    ErrorPayload {
        code: match error {
            AstrError::Cancelled | AstrError::LlmInterrupted => "cancelled",
            AstrError::Validation(_) => "validation_error",
            AstrError::Io { .. } => "io_error",
            AstrError::Parse { .. } => "parse_error",
            _ => "internal_error",
        }
        .to_string(),
        message: error.to_string(),
        details: Value::Null,
        retriable: error.is_retryable(),
    }
}

fn result_error_to_astr(result: ResultMessage) -> AstrError {
    let request_id = result.id;
    match result.error {
        Some(error) if error.code == "cancelled" => AstrError::Cancelled,
        Some(error) => AstrError::Internal(format!(
            "plugin request '{}' failed: {}",
            request_id, error.message
        )),
        None => AstrError::Internal(format!(
            "plugin request '{}' failed without error payload",
            request_id
        )),
    }
}
