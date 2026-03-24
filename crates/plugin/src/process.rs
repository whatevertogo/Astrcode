use std::process::Stdio;
use std::sync::Arc;

use astrcode_core::{AstrError, PluginManifest, Result};
use tokio::process::{Child, Command};

use crate::transport::{StdioTransport, Transport};

pub struct PluginProcess {
    pub manifest: PluginManifest,
    pub child: Child,
    transport: Arc<dyn Transport>,
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
        let transport: Arc<dyn Transport> = Arc::new(StdioTransport::from_child(stdin, stdout));

        Ok(Self {
            manifest: manifest.clone(),
            child,
            transport,
        })
    }

    pub fn transport(&self) -> Arc<dyn Transport> {
        Arc::clone(&self.transport)
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        match self.child.kill().await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => Ok(()),
            Err(error) => Err(AstrError::io("failed to terminate plugin process", error)),
        }
    }
}
