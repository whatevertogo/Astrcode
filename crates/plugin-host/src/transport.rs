use std::{fmt, pin::Pin};

use astrcode_core::{AstrError, Result};
use astrcode_protocol::plugin::{
    EventMessage, EventPhase, InitializeMessage, InitializeResultData, InvokeMessage,
    PluginMessage, ResultMessage,
};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout},
    sync::Mutex,
};

/// `plugin-host` 的最小 stdio 协议传输。
///
/// 它只负责 line-delimited JSON 消息收发，不承担 peer/read-loop。
/// 这样可以先把真实 transport owner 接到新边界里，再逐步补更复杂的并发协议语义。
pub struct PluginStdioTransport {
    writer: Mutex<Pin<Box<dyn AsyncWrite + Send>>>,
    reader: Mutex<Pin<Box<dyn AsyncBufRead + Send>>>,
}

impl fmt::Debug for PluginStdioTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PluginStdioTransport")
            .finish_non_exhaustive()
    }
}

impl PluginStdioTransport {
    pub fn from_child(stdin: ChildStdin, stdout: ChildStdout) -> Self {
        Self::from_streams(stdin, BufReader::new(stdout))
    }

    pub fn from_streams<W, R>(writer: W, reader: R) -> Self
    where
        W: AsyncWrite + Send + 'static,
        R: AsyncBufRead + Send + 'static,
    {
        Self {
            writer: Mutex::new(Box::pin(writer)),
            reader: Mutex::new(Box::pin(reader)),
        }
    }

    pub async fn initialize(&self, request: &InitializeMessage) -> Result<InitializeResultData> {
        self.send_message(&PluginMessage::Initialize(request.clone()))
            .await?;
        let response = self
            .recv_result_message(&request.id, Some("initialize"))
            .await?;
        if !response.success {
            return Err(result_message_error(response));
        }
        response.parse_output().map_err(|error| {
            AstrError::Validation(format!("failed to parse initialize result: {error}"))
        })
    }

    pub async fn invoke_unary(&self, request: &InvokeMessage) -> Result<ResultMessage> {
        self.send_message(&PluginMessage::Invoke(request.clone()))
            .await?;
        self.recv_result_message(&request.id, None).await
    }

    pub async fn invoke_stream(&self, request: &InvokeMessage) -> Result<Vec<EventMessage>> {
        self.send_message(&PluginMessage::Invoke(request.clone()))
            .await?;
        self.recv_stream_events(&request.id).await
    }

    async fn send_message(&self, message: &PluginMessage) -> Result<()> {
        let payload = serde_json::to_string(message).map_err(|error| {
            AstrError::Validation(format!("failed to serialize plugin message: {error}"))
        })?;
        let mut writer = self.writer.lock().await;
        writer
            .write_all(payload.as_bytes())
            .await
            .map_err(|error| AstrError::io("failed to write plugin payload", error))?;
        writer
            .write_all(b"\n")
            .await
            .map_err(|error| AstrError::io("failed to terminate plugin payload", error))?;
        writer
            .flush()
            .await
            .map_err(|error| AstrError::io("failed to flush plugin payload", error))
    }

    async fn recv_message(&self) -> Result<Option<PluginMessage>> {
        let mut reader = self.reader.lock().await;
        let mut line = String::new();
        let bytes = reader
            .read_line(&mut line)
            .await
            .map_err(|error| AstrError::io("failed to read plugin payload", error))?;
        if bytes == 0 {
            return Ok(None);
        }
        let payload = line.trim_end_matches(['\r', '\n']);
        serde_json::from_str(payload).map(Some).map_err(|error| {
            AstrError::Validation(format!("failed to decode plugin payload: {error}"))
        })
    }

    async fn recv_result_message(
        &self,
        request_id: &str,
        expected_kind: Option<&str>,
    ) -> Result<ResultMessage> {
        let message = self.recv_message().await?.ok_or_else(|| {
            AstrError::Internal(format!(
                "plugin transport closed before result for request '{request_id}'"
            ))
        })?;
        match message {
            PluginMessage::Result(result) if result.id == request_id => {
                if let Some(kind) = expected_kind {
                    if result.kind.as_deref() != Some(kind) {
                        return Err(AstrError::Internal(format!(
                            "expected result kind '{kind}' for request '{request_id}', got {:?}",
                            result.kind
                        )));
                    }
                }
                Ok(result)
            },
            PluginMessage::Event(event) if event.id == request_id => {
                Err(AstrError::Internal(format!(
                    "received event phase {:?} for unary request '{request_id}'",
                    event.phase
                )))
            },
            PluginMessage::Result(result) => Err(AstrError::Internal(format!(
                "received result for unexpected request '{}' while waiting for '{}'",
                result.id, request_id
            ))),
            PluginMessage::Event(event) => Err(AstrError::Internal(format!(
                "received event for unexpected request '{}' while waiting for '{}'",
                event.id, request_id
            ))),
            other => Err(AstrError::Internal(format!(
                "received unexpected plugin message {:?} while waiting for result '{}'",
                other, request_id
            ))),
        }
    }

    async fn recv_stream_events(&self, request_id: &str) -> Result<Vec<EventMessage>> {
        let mut events = Vec::new();
        loop {
            let message = self.recv_message().await?.ok_or_else(|| {
                AstrError::Internal(format!(
                    "plugin transport closed before stream request '{request_id}' completed"
                ))
            })?;
            match message {
                PluginMessage::Event(event) if event.id == request_id => {
                    let terminal =
                        matches!(event.phase, EventPhase::Completed | EventPhase::Failed);
                    events.push(event);
                    if terminal {
                        return Ok(events);
                    }
                },
                PluginMessage::Result(result) if result.id == request_id => {
                    return Err(AstrError::Internal(format!(
                        "received unary result for streaming request '{request_id}'"
                    )));
                },
                PluginMessage::Result(result) => {
                    return Err(AstrError::Internal(format!(
                        "received result for unexpected request '{}' while waiting for stream '{}'",
                        result.id, request_id
                    )));
                },
                PluginMessage::Event(event) => {
                    return Err(AstrError::Internal(format!(
                        "received event for unexpected request '{}' while waiting for stream '{}'",
                        event.id, request_id
                    )));
                },
                other => {
                    return Err(AstrError::Internal(format!(
                        "received unexpected plugin message {:?} while waiting for stream '{}'",
                        other, request_id
                    )));
                },
            }
        }
    }
}

fn result_message_error(result: ResultMessage) -> AstrError {
    let message = result
        .error
        .map(|error| error.message)
        .unwrap_or_else(|| "plugin invocation failed".to_string());
    AstrError::Validation(message)
}

#[cfg(test)]
mod tests {
    use astrcode_protocol::plugin::{
        CallerRef, CapabilityWireDescriptor, ErrorPayload, EventMessage, EventPhase,
        HandlerDescriptor, InitializeMessage, InitializeResultData, InvokeMessage, PeerDescriptor,
        PeerRole, PluginMessage, ProfileDescriptor, ResultMessage, TriggerDescriptor,
    };
    use serde_json::json;
    use tokio::io::{BufReader, duplex};

    use super::PluginStdioTransport;

    fn sample_initialize() -> InitializeMessage {
        InitializeMessage {
            id: "init-1".to_string(),
            protocol_version: "5".to_string(),
            supported_protocol_versions: vec!["5".to_string()],
            peer: PeerDescriptor {
                id: "host-1".to_string(),
                name: "plugin-host".to_string(),
                role: PeerRole::Supervisor,
                version: "0.1.0".to_string(),
                supported_profiles: vec!["coding".to_string()],
                metadata: json!({ "owner": "plugin-host" }),
            },
            capabilities: vec![
                CapabilityWireDescriptor::builder(
                    "tool.search",
                    astrcode_core::CapabilityKind::tool(),
                )
                .description("search workspace")
                .input_schema(json!({ "type": "object" }))
                .output_schema(json!({ "type": "object" }))
                .build()
                .expect("capability should build"),
            ],
            handlers: vec![HandlerDescriptor {
                id: "observe-tool-call".to_string(),
                trigger: TriggerDescriptor {
                    kind: "event".to_string(),
                    value: "tool_call".to_string(),
                    metadata: json!({ "source": "transport-test" }),
                },
                input_schema: json!({ "type": "object" }),
                profiles: vec!["coding".to_string()],
                filters: Vec::new(),
                permissions: Vec::new(),
            }],
            profiles: vec![ProfileDescriptor {
                name: "coding".to_string(),
                version: "1".to_string(),
                description: "coding profile".to_string(),
                context_schema: serde_json::Value::Null,
                metadata: serde_json::Value::Null,
            }],
            metadata: json!({ "test": true }),
        }
    }

    fn sample_invoke(stream: bool) -> InvokeMessage {
        InvokeMessage {
            id: "req-1".to_string(),
            capability: if stream {
                "tool.patch_stream".to_string()
            } else {
                "tool.search".to_string()
            },
            input: json!({ "query": "plugin-host" }),
            context: astrcode_protocol::plugin::InvocationContext {
                request_id: if stream {
                    "req-1-stream".to_string()
                } else {
                    "req-1-unary".to_string()
                },
                trace_id: None,
                session_id: Some("session-1".to_string()),
                caller: Some(CallerRef {
                    id: "test".to_string(),
                    role: "integration-test".to_string(),
                    metadata: serde_json::Value::Null,
                }),
                workspace: None,
                deadline_ms: None,
                budget: None,
                profile: "coding".to_string(),
                profile_context: serde_json::Value::Null,
                metadata: serde_json::Value::Null,
            },
            stream,
        }
    }

    async fn write_message(writer: &mut (impl AsyncWrite + Unpin), message: &PluginMessage) {
        let payload = serde_json::to_string(message).expect("message should serialize");
        writer
            .write_all(payload.as_bytes())
            .await
            .expect("write should succeed");
        writer
            .write_all(b"\n")
            .await
            .expect("newline should succeed");
        writer.flush().await.expect("flush should succeed");
    }

    async fn read_message(reader: &mut (impl AsyncBufRead + Unpin)) -> PluginMessage {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .expect("read should succeed");
        serde_json::from_str(line.trim_end()).expect("message should deserialize")
    }

    use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

    #[tokio::test]
    async fn stdio_transport_initializes_against_real_line_protocol() {
        let (client_side, server_side) = duplex(4096);
        let (client_reader, client_writer) = tokio::io::split(client_side);
        let (server_reader, server_writer) = tokio::io::split(server_side);
        let transport =
            PluginStdioTransport::from_streams(client_writer, BufReader::new(client_reader));

        let server = tokio::spawn(async move {
            let mut reader = BufReader::new(server_reader);
            let mut writer = server_writer;
            let message = read_message(&mut reader).await;
            let PluginMessage::Initialize(request) = message else {
                panic!("expected initialize message");
            };
            assert_eq!(request.id, "init-1");
            write_message(
                &mut writer,
                &PluginMessage::Result(ResultMessage {
                    id: request.id,
                    kind: Some("initialize".to_string()),
                    success: true,
                    output: serde_json::to_value(InitializeResultData {
                        protocol_version: "5".to_string(),
                        peer: PeerDescriptor {
                            id: "worker-1".to_string(),
                            name: "fixture".to_string(),
                            role: PeerRole::Worker,
                            version: "0.1.0".to_string(),
                            supported_profiles: vec!["coding".to_string()],
                            metadata: json!({ "fixture": true }),
                        },
                        capabilities: Vec::new(),
                        handlers: Vec::new(),
                        profiles: vec![ProfileDescriptor {
                            name: "coding".to_string(),
                            version: "1".to_string(),
                            description: "coding".to_string(),
                            context_schema: serde_json::Value::Null,
                            metadata: serde_json::Value::Null,
                        }],
                        skills: Vec::new(),
                        modes: Vec::new(),
                        metadata: serde_json::Value::Null,
                    })
                    .expect("initialize result should serialize"),
                    error: None,
                    metadata: serde_json::Value::Null,
                }),
            )
            .await;
        });

        let negotiated = transport
            .initialize(&sample_initialize())
            .await
            .expect("initialize should succeed");
        assert_eq!(negotiated.peer.id, "worker-1");
        server.await.expect("server should finish");
    }

    #[tokio::test]
    async fn stdio_transport_invokes_unary_request() {
        let (client_side, server_side) = duplex(4096);
        let (client_reader, client_writer) = tokio::io::split(client_side);
        let (server_reader, server_writer) = tokio::io::split(server_side);
        let transport =
            PluginStdioTransport::from_streams(client_writer, BufReader::new(client_reader));

        let server = tokio::spawn(async move {
            let mut reader = BufReader::new(server_reader);
            let mut writer = server_writer;
            let message = read_message(&mut reader).await;
            let PluginMessage::Invoke(request) = message else {
                panic!("expected invoke message");
            };
            assert!(!request.stream);
            write_message(
                &mut writer,
                &PluginMessage::Result(ResultMessage {
                    id: request.id,
                    kind: Some("tool_result".to_string()),
                    success: true,
                    output: json!({ "matches": 9 }),
                    error: None,
                    metadata: json!({ "transport": "stdio" }),
                }),
            )
            .await;
        });

        let result = transport
            .invoke_unary(&sample_invoke(false))
            .await
            .expect("unary invoke should succeed");
        assert!(result.success);
        assert_eq!(result.output, json!({ "matches": 9 }));
        server.await.expect("server should finish");
    }

    #[tokio::test]
    async fn stdio_transport_collects_stream_events_until_terminal() {
        let (client_side, server_side) = duplex(4096);
        let (client_reader, client_writer) = tokio::io::split(client_side);
        let (server_reader, server_writer) = tokio::io::split(server_side);
        let transport =
            PluginStdioTransport::from_streams(client_writer, BufReader::new(client_reader));

        let server = tokio::spawn(async move {
            let mut reader = BufReader::new(server_reader);
            let mut writer = server_writer;
            let message = read_message(&mut reader).await;
            let PluginMessage::Invoke(request) = message else {
                panic!("expected invoke message");
            };
            assert!(request.stream);
            for (seq, phase, payload) in [
                (0, EventPhase::Started, json!({ "status": "started" })),
                (1, EventPhase::Delta, json!({ "chunk": 1 })),
                (2, EventPhase::Completed, json!({ "status": "done" })),
            ] {
                write_message(
                    &mut writer,
                    &PluginMessage::Event(EventMessage {
                        id: request.id.clone(),
                        phase,
                        event: "artifact.patch".to_string(),
                        payload,
                        seq,
                        error: None,
                    }),
                )
                .await;
            }
        });

        let events = transport
            .invoke_stream(&sample_invoke(true))
            .await
            .expect("stream invoke should succeed");
        assert_eq!(events.len(), 3);
        assert_eq!(events[1].phase, EventPhase::Delta);
        assert_eq!(events[2].phase, EventPhase::Completed);
        server.await.expect("server should finish");
    }

    #[tokio::test]
    async fn stdio_transport_rejects_mismatched_request_ids() {
        let (client_side, server_side) = duplex(4096);
        let (client_reader, client_writer) = tokio::io::split(client_side);
        let (server_reader, server_writer) = tokio::io::split(server_side);
        let transport =
            PluginStdioTransport::from_streams(client_writer, BufReader::new(client_reader));

        let server = tokio::spawn(async move {
            let mut reader = BufReader::new(server_reader);
            let mut writer = server_writer;
            let message = read_message(&mut reader).await;
            let PluginMessage::Invoke(_) = message else {
                panic!("expected invoke message");
            };
            write_message(
                &mut writer,
                &PluginMessage::Result(ResultMessage {
                    id: "other-request".to_string(),
                    kind: Some("tool_result".to_string()),
                    success: false,
                    output: serde_json::Value::Null,
                    error: Some(ErrorPayload {
                        code: "unexpected".to_string(),
                        message: "wrong request".to_string(),
                        details: serde_json::Value::Null,
                        retriable: false,
                    }),
                    metadata: serde_json::Value::Null,
                }),
            )
            .await;
        });

        let error = transport
            .invoke_unary(&sample_invoke(false))
            .await
            .expect_err("mismatched request id should fail");
        assert!(error.to_string().contains("unexpected request"));
        server.await.expect("server should finish");
    }
}
