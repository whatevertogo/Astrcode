use std::pin::Pin;

use async_trait::async_trait;
use tokio::io::{self, AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::Mutex;

use super::Transport;

pub struct StdioTransport {
    writer: Mutex<Pin<Box<dyn AsyncWrite + Send>>>,
    reader: Mutex<Pin<Box<dyn AsyncBufRead + Send>>>,
}

impl StdioTransport {
    pub fn from_child(stdin: ChildStdin, stdout: ChildStdout) -> Self {
        Self {
            writer: Mutex::new(Box::pin(stdin)),
            reader: Mutex::new(Box::pin(BufReader::new(stdout))),
        }
    }

    pub fn from_process_stdio() -> Self {
        Self {
            writer: Mutex::new(Box::pin(io::stdout())),
            reader: Mutex::new(Box::pin(BufReader::new(io::stdin()))),
        }
    }
}

#[async_trait]
impl Transport for StdioTransport {
    async fn send(&self, payload: &str) -> Result<(), String> {
        let mut writer = self.writer.lock().await;
        writer
            .write_all(payload.as_bytes())
            .await
            .map_err(|error| format!("failed to write plugin payload: {error}"))?;
        writer
            .write_all(b"\n")
            .await
            .map_err(|error| format!("failed to terminate plugin payload: {error}"))?;
        writer
            .flush()
            .await
            .map_err(|error| format!("failed to flush plugin payload: {error}"))
    }

    async fn recv(&self) -> Result<Option<String>, String> {
        let mut reader = self.reader.lock().await;
        let mut line = String::new();
        let bytes = reader
            .read_line(&mut line)
            .await
            .map_err(|error| format!("failed to read plugin payload: {error}"))?;
        if bytes == 0 {
            return Ok(None);
        }
        Ok(Some(line.trim_end_matches(['\r', '\n']).to_string()))
    }
}
