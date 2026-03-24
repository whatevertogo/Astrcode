use std::error::Error;
use std::future::Future;
use std::pin::Pin;

use astrcode_protocol::plugin::CapabilityDescriptor;
use serde_json::Value;

use crate::{PluginContext, StreamWriter};

pub type ToolResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

pub trait ToolHandler: Send + Sync {
    fn descriptor(&self) -> CapabilityDescriptor;

    fn execute(
        &self,
        input: Value,
        context: PluginContext,
        stream: StreamWriter,
    ) -> Pin<Box<dyn Future<Output = ToolResult<Value>> + Send>>;
}

pub struct ToolRegistration {
    descriptor: CapabilityDescriptor,
    handler: Box<dyn ToolHandler>,
}

impl ToolRegistration {
    pub fn new(handler: Box<dyn ToolHandler>) -> Self {
        let descriptor = handler.descriptor();
        Self {
            descriptor,
            handler,
        }
    }

    pub fn descriptor(&self) -> &CapabilityDescriptor {
        &self.descriptor
    }

    pub fn handler(&self) -> &dyn ToolHandler {
        self.handler.as_ref()
    }
}
