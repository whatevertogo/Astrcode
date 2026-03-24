use astrcode_core::{AstrError, Result};
use astrcode_protocol::plugin::{InvokeRequest, InvokeResult, PluginMessage};

use crate::transport::Transport;

pub struct PluginExecutor<'a> {
    transport: &'a mut dyn Transport,
}

impl<'a> PluginExecutor<'a> {
    pub fn new(transport: &'a mut dyn Transport) -> Self {
        Self { transport }
    }

    pub async fn invoke(&mut self, req: InvokeRequest) -> Result<InvokeResult> {
        self.transport.send(&PluginMessage::Invoke(req)).await?;
        match self.transport.receive().await? {
            PluginMessage::Result(result) => Ok(result),
            PluginMessage::Event(_) => Err(AstrError::Internal(
                "unexpected stream event without terminal result".to_string(),
            )),
            other => Err(AstrError::Internal(format!(
                "unexpected plugin invoke response: {other:?}"
            ))),
        }
    }
}
