use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Mutex;

use astrcode_core::{ManagedRuntimeComponent, PluginManifest, Result};
use astrcode_protocol::plugin::{
    CapabilityDescriptor, InitializeMessage, InitializeResultData, InvokeMessage, PeerDescriptor,
    ProfileDescriptor, ResultMessage, PROTOCOL_VERSION,
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::core_to_protocol_capability;
use crate::{CapabilityRouter, Peer, PluginProcess, StreamExecution};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupervisorHealth {
    Healthy,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupervisorHealthReport {
    pub health: SupervisorHealth,
    pub message: Option<String>,
}

pub struct Supervisor {
    manifest_name: String,
    process: Mutex<PluginProcess>,
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
        let mut process = process;
        let manifest_name = process.manifest.name.clone();
        let router = Arc::new(CapabilityRouter::default());
        let initialize = local_initialize.unwrap_or_else(|| {
            default_initialize_message(local_peer, Vec::new(), default_profiles())
        });
        let peer = Peer::new(process.transport(), initialize, router);
        let remote_initialize = match peer.initialize().await {
            Ok(remote_initialize) => remote_initialize,
            Err(error) => {
                if let Err(shutdown_error) = process.shutdown().await {
                    log::warn!(
                        "failed to terminate plugin '{}' after initialize error: {}",
                        manifest_name,
                        shutdown_error
                    );
                }
                return Err(error);
            }
        };
        Ok(Self {
            manifest_name,
            process: Mutex::new(process),
            peer,
            remote_initialize,
        })
    }

    pub fn remote_initialize(&self) -> &InitializeResultData {
        &self.remote_initialize
    }

    pub(crate) fn peer(&self) -> Peer {
        self.peer.clone()
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

    pub async fn shutdown(&self) -> Result<()> {
        self.process.lock().await.shutdown().await
    }

    pub async fn health_report(&self) -> Result<SupervisorHealthReport> {
        if let Some(reason) = self.peer.closed_reason().await {
            return Ok(SupervisorHealthReport {
                health: SupervisorHealth::Unavailable,
                message: Some(format!("protocol peer closed: {reason}")),
            });
        }

        let status = self.process.lock().await.status()?;
        if status.running {
            Ok(SupervisorHealthReport {
                health: SupervisorHealth::Healthy,
                message: None,
            })
        } else {
            Ok(SupervisorHealthReport {
                health: SupervisorHealth::Unavailable,
                message: Some(match status.exit_code {
                    Some(code) => format!("plugin process exited with code {code}"),
                    None => "plugin process exited".to_string(),
                }),
            })
        }
    }
}

#[async_trait]
impl ManagedRuntimeComponent for Supervisor {
    fn component_name(&self) -> String {
        format!("plugin supervisor '{}'", self.manifest_name)
    }

    async fn shutdown_component(&self) -> Result<()> {
        self.shutdown().await
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
        .map(core_to_protocol_capability)
        .collect()
}
