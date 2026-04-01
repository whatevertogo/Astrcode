use std::sync::Arc;

use astrcode_core::Result;
use astrcode_protocol::plugin::{InitializeMessage, PeerDescriptor};

use crate::transport::StdioTransport;
use crate::{CapabilityRouter, Peer};

/// 进程内插件 Worker—— 通过 stdio 与宿主进程通信。
///
/// 通常用于子进程模式下，插件二进制通过 `Worker::from_stdio()` 创建连接，
/// 然后进入 `run()` 循环直到连接关闭。
pub struct Worker {
    peer: Peer,
}

impl Worker {
    pub fn from_stdio(
        local_peer: PeerDescriptor,
        router: CapabilityRouter,
        local_initialize: Option<InitializeMessage>,
    ) -> Self {
        let capabilities = router.capabilities();
        let initialize = local_initialize.unwrap_or_else(|| {
            crate::supervisor::default_initialize_message(
                local_peer,
                capabilities,
                crate::supervisor::default_profiles(),
            )
        });
        let transport = Arc::new(StdioTransport::from_process_stdio());
        let peer = Peer::new(transport, initialize, Arc::new(router));
        Self { peer }
    }

    pub async fn run(&self) -> Result<()> {
        self.peer.wait_closed().await;
        Ok(())
    }
}
