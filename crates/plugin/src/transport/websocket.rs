use async_trait::async_trait;

use super::Transport;

#[derive(Debug, Default)]
pub struct WebSocketTransport;

#[async_trait]
impl Transport for WebSocketTransport {
    async fn send(&self, _payload: &str) -> Result<(), String> {
        Err("websocket transport is not implemented yet".to_string())
    }

    async fn recv(&self) -> Result<Option<String>, String> {
        Err("websocket transport is not implemented yet".to_string())
    }
}
