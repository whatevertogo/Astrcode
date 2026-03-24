mod stdio;
mod websocket;

use astrcode_core::Result;
use astrcode_protocol::plugin::PluginMessage;
use async_trait::async_trait;

pub use stdio::StdioTransport;
pub use websocket::WebSocketTransport;

#[async_trait]
pub trait Transport: Send {
    async fn send(&mut self, message: &PluginMessage) -> Result<()>;
    async fn receive(&mut self) -> Result<PluginMessage>;
}
