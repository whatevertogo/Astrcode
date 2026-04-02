use async_trait::async_trait;

use crate::AstrError;

#[async_trait]
pub trait RuntimeHandle: Send + Sync {
    fn runtime_name(&self) -> &'static str;

    fn runtime_kind(&self) -> &'static str;

    async fn shutdown(&self, timeout_secs: u64) -> std::result::Result<(), AstrError>;
}

#[async_trait]
pub trait ManagedRuntimeComponent: Send + Sync {
    fn component_name(&self) -> String;

    async fn shutdown_component(&self) -> std::result::Result<(), AstrError>;
}
