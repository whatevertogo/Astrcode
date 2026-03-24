use std::sync::Arc;
use std::time::Instant;

use astrcode_core::{
    AstrError, CapabilityContext, CapabilityDescriptor as CoreCapabilityDescriptor,
    CapabilityExecutionResult, CapabilityInvoker, CapabilityKind as CoreCapabilityKind,
    PermissionHint as CorePermissionHint, Result, SideEffectLevel as CoreSideEffectLevel,
    StabilityLevel as CoreStabilityLevel,
};
use astrcode_protocol::plugin::{
    CapabilityDescriptor, CapabilityKind, EventPhase, InvocationContext, PermissionHint,
    SideEffectLevel, StabilityLevel, WorkspaceRef,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{Peer, StreamExecution, Supervisor};

#[derive(Clone)]
pub struct PluginCapabilityInvoker {
    peer: Peer,
    descriptor: CoreCapabilityDescriptor,
    remote_name: String,
}

impl PluginCapabilityInvoker {
    pub fn from_protocol_descriptor(peer: Peer, descriptor: CapabilityDescriptor) -> Self {
        Self {
            remote_name: descriptor.name.clone(),
            descriptor: protocol_to_core_capability(&descriptor),
            peer,
        }
    }
}

#[async_trait]
impl CapabilityInvoker for PluginCapabilityInvoker {
    fn descriptor(&self) -> CoreCapabilityDescriptor {
        self.descriptor.clone()
    }

    async fn invoke(
        &self,
        payload: Value,
        ctx: &CapabilityContext,
    ) -> Result<CapabilityExecutionResult> {
        let started_at = Instant::now();
        let invocation = to_invocation_context(ctx);

        if self.descriptor.streaming {
            let mut stream = self
                .peer
                .invoke_stream(astrcode_protocol::plugin::InvokeMessage {
                    id: invocation.request_id.clone(),
                    capability: self.remote_name.clone(),
                    input: payload,
                    context: invocation,
                    stream: true,
                })
                .await?;
            finish_stream_invocation(self.descriptor.name.clone(), &mut stream, started_at).await
        } else {
            let result = self
                .peer
                .invoke(astrcode_protocol::plugin::InvokeMessage {
                    id: invocation.request_id.clone(),
                    capability: self.remote_name.clone(),
                    input: payload,
                    context: invocation,
                    stream: false,
                })
                .await?;
            if result.success {
                Ok(CapabilityExecutionResult {
                    capability_name: self.descriptor.name.clone(),
                    success: true,
                    output: result.output,
                    error: None,
                    metadata: Some(result.metadata),
                    duration_ms: started_at.elapsed().as_millis(),
                    truncated: false,
                })
            } else {
                let error = result
                    .error
                    .map(|value| value.message)
                    .unwrap_or_else(|| "plugin invocation failed".to_string());
                Ok(CapabilityExecutionResult {
                    capability_name: self.descriptor.name.clone(),
                    success: false,
                    output: result.output,
                    error: Some(error),
                    metadata: Some(result.metadata),
                    duration_ms: started_at.elapsed().as_millis(),
                    truncated: false,
                })
            }
        }
    }
}

impl Supervisor {
    pub fn capability_invokers(&self) -> Vec<Arc<dyn CapabilityInvoker>> {
        self.remote_initialize()
            .capabilities
            .iter()
            .cloned()
            .map(|descriptor| {
                Arc::new(PluginCapabilityInvoker::from_protocol_descriptor(
                    self.peer(),
                    descriptor,
                )) as Arc<dyn CapabilityInvoker>
            })
            .collect()
    }

    pub fn core_capabilities(&self) -> Vec<CoreCapabilityDescriptor> {
        self.remote_initialize()
            .capabilities
            .iter()
            .map(protocol_to_core_capability)
            .collect()
    }
}

async fn finish_stream_invocation(
    capability_name: String,
    stream: &mut StreamExecution,
    started_at: Instant,
) -> Result<CapabilityExecutionResult> {
    let mut deltas = Vec::new();

    while let Some(event) = stream.recv().await {
        match event.phase {
            EventPhase::Started => {}
            EventPhase::Delta => {
                deltas.push(json!({
                    "event": event.event,
                    "payload": event.payload,
                    "seq": event.seq,
                }));
            }
            EventPhase::Completed => {
                return Ok(CapabilityExecutionResult {
                    capability_name,
                    success: true,
                    output: event.payload,
                    error: None,
                    metadata: Some(json!({ "streamEvents": deltas })),
                    duration_ms: started_at.elapsed().as_millis(),
                    truncated: false,
                });
            }
            EventPhase::Failed => {
                return Ok(CapabilityExecutionResult {
                    capability_name,
                    success: false,
                    output: Value::Null,
                    error: Some(
                        event
                            .error
                            .map(|value| value.message)
                            .unwrap_or_else(|| "stream invocation failed".to_string()),
                    ),
                    metadata: Some(json!({ "streamEvents": deltas })),
                    duration_ms: started_at.elapsed().as_millis(),
                    truncated: false,
                });
            }
        }
    }

    Err(AstrError::Internal(
        "plugin stream ended without terminal event".to_string(),
    ))
}

fn to_invocation_context(ctx: &CapabilityContext) -> InvocationContext {
    let working_dir = ctx.working_dir.to_string_lossy().into_owned();
    InvocationContext {
        request_id: ctx
            .request_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        trace_id: ctx.trace_id.clone(),
        session_id: Some(ctx.session_id.clone()),
        caller: None,
        workspace: Some(WorkspaceRef {
            working_dir: Some(working_dir.clone()),
            repo_root: Some(working_dir),
            branch: None,
            metadata: Value::Null,
        }),
        deadline_ms: None,
        budget: None,
        profile: ctx.profile.clone(),
        profile_context: ctx.profile_context.clone(),
        metadata: ctx.metadata.clone(),
    }
}

pub fn protocol_to_core_capability(descriptor: &CapabilityDescriptor) -> CoreCapabilityDescriptor {
    CoreCapabilityDescriptor {
        name: descriptor.name.clone(),
        kind: match descriptor.kind {
            CapabilityKind::Tool => CoreCapabilityKind::Tool,
            CapabilityKind::Agent => CoreCapabilityKind::Agent,
            CapabilityKind::ContextProvider => CoreCapabilityKind::ContextProvider,
            CapabilityKind::MemoryProvider => CoreCapabilityKind::MemoryProvider,
            CapabilityKind::PolicyHook => CoreCapabilityKind::PolicyHook,
            CapabilityKind::Renderer => CoreCapabilityKind::Renderer,
            CapabilityKind::Resource => CoreCapabilityKind::Resource,
        },
        description: descriptor.description.clone(),
        input_schema: descriptor.input_schema.clone(),
        output_schema: descriptor.output_schema.clone(),
        streaming: descriptor.streaming,
        profiles: descriptor.profiles.clone(),
        tags: descriptor.tags.clone(),
        permissions: descriptor
            .permissions
            .iter()
            .map(|permission| CorePermissionHint {
                name: permission.name.clone(),
                rationale: permission.rationale.clone(),
            })
            .collect(),
        side_effect: match descriptor.side_effect {
            SideEffectLevel::None => CoreSideEffectLevel::None,
            SideEffectLevel::Local => CoreSideEffectLevel::Local,
            SideEffectLevel::Workspace => CoreSideEffectLevel::Workspace,
            SideEffectLevel::External => CoreSideEffectLevel::External,
        },
        stability: match descriptor.stability {
            StabilityLevel::Experimental => CoreStabilityLevel::Experimental,
            StabilityLevel::Stable => CoreStabilityLevel::Stable,
            StabilityLevel::Deprecated => CoreStabilityLevel::Deprecated,
        },
    }
}

pub fn core_to_protocol_capability(
    capability: &astrcode_core::CapabilityDescriptor,
) -> CapabilityDescriptor {
    CapabilityDescriptor {
        name: capability.name.clone(),
        kind: match capability.kind {
            astrcode_core::CapabilityKind::Tool => CapabilityKind::Tool,
            astrcode_core::CapabilityKind::Agent => CapabilityKind::Agent,
            astrcode_core::CapabilityKind::ContextProvider => CapabilityKind::ContextProvider,
            astrcode_core::CapabilityKind::MemoryProvider => CapabilityKind::MemoryProvider,
            astrcode_core::CapabilityKind::PolicyHook => CapabilityKind::PolicyHook,
            astrcode_core::CapabilityKind::Renderer => CapabilityKind::Renderer,
            astrcode_core::CapabilityKind::Resource => CapabilityKind::Resource,
        },
        description: capability.description.clone(),
        input_schema: capability.input_schema.clone(),
        output_schema: capability.output_schema.clone(),
        streaming: capability.streaming,
        profiles: capability.profiles.clone(),
        tags: capability.tags.clone(),
        permissions: capability
            .permissions
            .iter()
            .map(|permission| PermissionHint {
                name: permission.name.clone(),
                rationale: permission.rationale.clone(),
            })
            .collect(),
        side_effect: match capability.side_effect {
            astrcode_core::SideEffectLevel::None => SideEffectLevel::None,
            astrcode_core::SideEffectLevel::Local => SideEffectLevel::Local,
            astrcode_core::SideEffectLevel::Workspace => SideEffectLevel::Workspace,
            astrcode_core::SideEffectLevel::External => SideEffectLevel::External,
        },
        stability: match capability.stability {
            astrcode_core::StabilityLevel::Experimental => StabilityLevel::Experimental,
            astrcode_core::StabilityLevel::Stable => StabilityLevel::Stable,
            astrcode_core::StabilityLevel::Deprecated => StabilityLevel::Deprecated,
        },
    }
}
