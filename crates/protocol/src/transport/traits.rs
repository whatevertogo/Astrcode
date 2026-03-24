use async_trait::async_trait;

#[async_trait]
pub trait Transport: Send + Sync {
    async fn send(&self, payload: &str) -> Result<(), String>;
    async fn recv(&self) -> Result<Option<String>, String>;
}
