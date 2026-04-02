use std::time::Duration;

use astrcode_core::{AstrError, CancelToken, Result};
use astrcode_plugin::{CapabilityHandler, CapabilityRouter, EventEmitter, Worker};
use astrcode_protocol::plugin::{
    CapabilityDescriptor, CapabilityKind, InvocationContext, PeerDescriptor, PeerRole,
    SideEffectLevel, StabilityLevel,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::time::sleep;

struct EchoHandler;

#[async_trait]
impl CapabilityHandler for EchoHandler {
    fn descriptor(&self) -> CapabilityDescriptor {
        CapabilityDescriptor {
            name: "tool.echo".to_string(),
            kind: CapabilityKind::tool(),
            description: "Echo the input".to_string(),
            input_schema: json!({ "type": "object" }),
            output_schema: json!({ "type": "object" }),
            streaming: false,
            concurrency_safe: false,
            compact_clearable: false,
            profiles: vec!["coding".to_string()],
            tags: vec!["fixture".to_string()],
            permissions: vec![],
            side_effect: SideEffectLevel::None,
            stability: StabilityLevel::Stable,
            metadata: Value::Null,
        }
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
    fn descriptor(&self) -> CapabilityDescriptor {
        CapabilityDescriptor {
            name: "tool.patch_stream".to_string(),
            kind: CapabilityKind::tool(),
            description: "Emit patch deltas".to_string(),
            input_schema: json!({ "type": "object" }),
            output_schema: json!({ "type": "object" }),
            streaming: true,
            concurrency_safe: false,
            compact_clearable: false,
            profiles: vec!["coding".to_string()],
            tags: vec!["fixture".to_string(), "stream".to_string()],
            permissions: vec![],
            side_effect: SideEffectLevel::Workspace,
            stability: StabilityLevel::Stable,
            metadata: Value::Null,
        }
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
    fn descriptor(&self) -> CapabilityDescriptor {
        CapabilityDescriptor {
            name: "tool.delayed_echo".to_string(),
            kind: CapabilityKind::tool(),
            description: "Delay before returning".to_string(),
            input_schema: json!({ "type": "object" }),
            output_schema: json!({ "type": "object" }),
            streaming: false,
            concurrency_safe: false,
            compact_clearable: false,
            profiles: vec!["coding".to_string()],
            tags: vec!["fixture".to_string(), "delayed".to_string()],
            permissions: vec![],
            side_effect: SideEffectLevel::None,
            stability: StabilityLevel::Stable,
            metadata: Value::Null,
        }
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
    );
    worker.run().await
}
