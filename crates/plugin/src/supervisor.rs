use std::sync::Arc;

use astrcode_core::{PluginManifest, Result};
use astrcode_protocol::plugin::{
    CapabilityDescriptor, CapabilityKind, InitializeMessage, InitializeResultData, InvokeMessage,
    PeerDescriptor, ProfileDescriptor, ResultMessage, SideEffectLevel, StabilityLevel,
    PROTOCOL_VERSION,
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::{CapabilityRouter, Peer, PluginProcess, StreamExecution};

pub struct Supervisor {
    process: PluginProcess,
    peer: Peer,
    remote_initialize: InitializeResultData,
}

impl Supervisor {
    pub async fn start(manifest: &PluginManifest, local_peer: PeerDescriptor) -> Result<Self> {
        let process = PluginProcess::start(manifest).await?;
        Self::from_process(process, local_peer, None).await
    }

    pub async fn from_process(
        process: PluginProcess,
        local_peer: PeerDescriptor,
        local_initialize: Option<InitializeMessage>,
    ) -> Result<Self> {
        let router = Arc::new(CapabilityRouter::default());
        let initialize = local_initialize.unwrap_or_else(|| {
            default_initialize_message(local_peer, Vec::new(), default_profiles())
        });
        let peer = Peer::new(process.transport(), initialize, router);
        let remote_initialize = peer.initialize().await?;
        Ok(Self {
            process,
            peer,
            remote_initialize,
        })
    }

    pub fn remote_initialize(&self) -> &InitializeResultData {
        &self.remote_initialize
    }

    pub async fn invoke(
        &self,
        capability: impl Into<String>,
        input: Value,
        context: astrcode_protocol::plugin::InvocationContext,
    ) -> Result<ResultMessage> {
        self.peer
            .invoke(InvokeMessage {
                id: Uuid::new_v4().to_string(),
                capability: capability.into(),
                input,
                context,
                stream: false,
            })
            .await
    }

    pub async fn invoke_stream(
        &self,
        capability: impl Into<String>,
        input: Value,
        context: astrcode_protocol::plugin::InvocationContext,
    ) -> Result<StreamExecution> {
        self.peer
            .invoke_stream(InvokeMessage {
                id: Uuid::new_v4().to_string(),
                capability: capability.into(),
                input,
                context,
                stream: true,
            })
            .await
    }

    pub async fn cancel(
        &self,
        request_id: impl Into<String>,
        reason: Option<String>,
    ) -> Result<()> {
        self.peer.cancel(request_id, reason).await
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        self.process.shutdown().await
    }
}

pub fn default_initialize_message(
    local_peer: PeerDescriptor,
    capabilities: Vec<CapabilityDescriptor>,
    profiles: Vec<ProfileDescriptor>,
) -> InitializeMessage {
    InitializeMessage {
        id: Uuid::new_v4().to_string(),
        protocol_version: PROTOCOL_VERSION.to_string(),
        supported_protocol_versions: vec![PROTOCOL_VERSION.to_string()],
        peer: local_peer,
        capabilities,
        handlers: Vec::new(),
        profiles,
        metadata: json!({ "transport": "stdio" }),
    }
}

pub fn default_profiles() -> Vec<ProfileDescriptor> {
    vec![ProfileDescriptor {
        name: "coding".to_string(),
        version: "1".to_string(),
        description: "Coding workflow profile".to_string(),
        context_schema: json!({
            "type": "object",
            "properties": {
                "workingDir": { "type": "string" },
                "repoRoot": { "type": "string" },
                "openFiles": { "type": "array", "items": { "type": "string" } },
                "activeFile": { "type": "string" },
                "selection": { "type": "object" },
                "approvalMode": { "type": "string" }
            }
        }),
        metadata: Value::Null,
    }]
}

pub fn manifest_capabilities(manifest: &PluginManifest) -> Vec<CapabilityDescriptor> {
    manifest
        .capabilities
        .iter()
        .map(|capability| CapabilityDescriptor {
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
                .map(|permission| astrcode_protocol::plugin::PermissionHint {
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
        })
        .collect()
}
