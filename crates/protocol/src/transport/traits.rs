use async_trait::async_trait;

use crate::plugin::PluginMessage;

#[async_trait]
pub trait Transport: Send + Sync {
    async fn send(&mut self, message: &PluginMessage) -> Result<(), String>;
    async fn recv(&mut self) -> Result<PluginMessage, String>;
}
