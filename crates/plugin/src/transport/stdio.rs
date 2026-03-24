use astrcode_core::{AstrError, Result};
use astrcode_protocol::plugin::PluginMessage;
use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};

use super::Transport;

pub struct StdioTransport {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl StdioTransport {
    pub fn new(stdin: ChildStdin, stdout: ChildStdout) -> Self {
        Self {
            stdin,
            stdout: BufReader::new(stdout),
        }
    }
}

#[async_trait]
impl Transport for StdioTransport {
    async fn send(&mut self, message: &PluginMessage) -> Result<()> {
        let json = serde_json::to_string(message)
            .map_err(|error| AstrError::parse("failed to serialize plugin message", error))?;
        self.stdin
            .write_all(json.as_bytes())
            .await
            .map_err(|error| AstrError::io("failed to write plugin request", error))?;
        self.stdin
            .write_all(b"\n")
            .await
            .map_err(|error| AstrError::io("failed to terminate plugin request", error))
    }

    async fn receive(&mut self) -> Result<PluginMessage> {
        let mut line = String::new();
        self.stdout
            .read_line(&mut line)
            .await
            .map_err(|error| AstrError::io("failed to read plugin response", error))?;
        serde_json::from_str(line.trim())
            .map_err(|error| AstrError::parse("failed to deserialize plugin response", error))
    }
}
