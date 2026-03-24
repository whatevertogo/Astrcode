use astrcode_core::{AstrError, Result};
use astrcode_protocol::plugin::{ClientInfo, InitializeRequest, InitializeResult, PluginMessage};

use crate::transport::Transport;

pub async fn perform_handshake(transport: &mut dyn Transport) -> Result<InitializeResult> {
    let request = PluginMessage::Initialize(InitializeRequest {
        protocol_version: "1".to_string(),
        client_info: ClientInfo {
            name: "astrcode-plugin-runtime".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    });
    transport.send(&request).await?;
    match transport.receive().await? {
        PluginMessage::InitializeResult(result) => Ok(result),
        other => Err(AstrError::Internal(format!(
            "unexpected plugin handshake response: {other:?}"
        ))),
    }
}
