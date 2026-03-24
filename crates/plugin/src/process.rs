use std::process::Stdio;

use astrcode_core::{AstrError, PluginManifest, Result};
use astrcode_protocol::plugin::{InitializeResult, InvokeRequest, InvokeResult};
use tokio::process::{Child, Command};

use crate::transport::{StdioTransport, Transport};

pub struct PluginProcess {
    pub manifest: PluginManifest,
    pub child: Child,
    pub transport: Box<dyn Transport>,
}

impl PluginProcess {
    pub async fn start(manifest: &PluginManifest) -> Result<Self> {
        let executable = manifest.executable.as_ref().ok_or_else(|| {
            AstrError::Validation(format!("plugin '{}' has no executable", manifest.name))
        })?;
        let mut child = Command::new(executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|error| {
                AstrError::io(format!("failed to spawn plugin '{executable}'"), error)
            })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            AstrError::Internal(format!("plugin '{}' did not expose stdin", manifest.name))
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            AstrError::Internal(format!("plugin '{}' did not expose stdout", manifest.name))
        })?;
        let transport = StdioTransport::new(stdin, stdout);

        Ok(Self {
            manifest: manifest.clone(),
            child,
            transport: Box::new(transport),
        })
    }

    pub async fn initialize(&mut self) -> Result<InitializeResult> {
        crate::handshake::perform_handshake(self.transport.as_mut()).await
    }

    pub async fn invoke(&mut self, req: InvokeRequest) -> Result<InvokeResult> {
        crate::executor::PluginExecutor::new(self.transport.as_mut())
            .invoke(req)
            .await
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        self.child
            .kill()
            .await
            .map_err(|error| AstrError::io("failed to terminate plugin process", error))
    }
}
