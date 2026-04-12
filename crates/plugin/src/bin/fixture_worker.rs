use std::time::Duration;

use astrcode_core::{
    AstrError, CancelToken, CapabilityKind, CapabilitySpec, InvocationMode, Result, SideEffect,
};
use astrcode_plugin::{CapabilityHandler, CapabilityRouter, EventEmitter, Worker};
use astrcode_protocol::plugin::{InvocationContext, PeerDescriptor, PeerRole};
use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::time::sleep;

struct EchoHandler;

#[async_trait]
impl CapabilityHandler for EchoHandler {
    fn capability_spec(&self) -> CapabilitySpec {
        CapabilitySpec::builder("tool.echo", CapabilityKind::Tool)
            .description("Echo the input")
            .schema(json!({ "type": "object" }), json!({ "type": "object" }))
            .profiles(["coding"])
            .tags(["fixture"])
            .build()
            .expect("fixture capability spec should build")
    }

    async fn invoke(
        &self,
        input: Value,
        _context: InvocationContext,
        _events: EventEmitter,
        _cancel: CancelToken,
    ) -> Result<Value> {
        Ok(input)
    }
}

struct PatchStreamHandler;

#[async_trait]
impl CapabilityHandler for PatchStreamHandler {
    fn capability_spec(&self) -> CapabilitySpec {
        CapabilitySpec::builder("tool.patch_stream", CapabilityKind::Tool)
            .description("Emit patch deltas")
            .schema(json!({ "type": "object" }), json!({ "type": "object" }))
            .invocation_mode(InvocationMode::Streaming)
            .profiles(["coding"])
            .tags(["fixture", "stream"])
            .side_effect(SideEffect::Workspace)
            .build()
            .expect("fixture capability spec should build")
    }

    async fn invoke(
        &self,
        input: Value,
        _context: InvocationContext,
        events: EventEmitter,
        cancel: CancelToken,
    ) -> Result<Value> {
        let path = input
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("src/main.rs");
        for chunk in 0..3_u64 {
            if cancel.is_cancelled() {
                return Err(AstrError::Cancelled);
            }
            events
                .delta(
                    "artifact.patch",
                    json!({
                        "path": path,
                        "chunk": chunk,
                        "patch": format!("@@ chunk {chunk} @@"),
                    }),
                )
                .await?;
            sleep(Duration::from_millis(40)).await;
        }

        if cancel.is_cancelled() {
            return Err(AstrError::Cancelled);
        }

        Ok(json!({
            "path": path,
            "chunks": 3,
            "status": "applied"
        }))
    }
}

struct DelayedEchoHandler;

#[async_trait]
impl CapabilityHandler for DelayedEchoHandler {
    fn capability_spec(&self) -> CapabilitySpec {
        CapabilitySpec::builder("tool.delayed_echo", CapabilityKind::Tool)
            .description("Delay before returning")
            .schema(json!({ "type": "object" }), json!({ "type": "object" }))
            .profiles(["coding"])
            .tags(["fixture", "delayed"])
            .build()
            .expect("fixture capability spec should build")
    }

    async fn invoke(
        &self,
        input: Value,
        _context: InvocationContext,
        _events: EventEmitter,
        cancel: CancelToken,
    ) -> Result<Value> {
        sleep(Duration::from_millis(300)).await;
        if cancel.is_cancelled() {
            return Err(AstrError::Cancelled);
        }
        Ok(input)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut router = CapabilityRouter::default();
    router.register(EchoHandler)?;
    router.register(PatchStreamHandler)?;
    router.register(DelayedEchoHandler)?;

    let worker = Worker::from_stdio(
        PeerDescriptor {
            id: "fixture-worker".to_string(),
            name: "fixture-worker".to_string(),
            role: PeerRole::Worker,
            version: env!("CARGO_PKG_VERSION").to_string(),
            supported_profiles: vec!["coding".to_string()],
            metadata: json!({ "fixture": true }),
        },
        router,
        None,
    )?;
    worker.run().await
}
