//! 插件 Worker—— 插件进程侧的入口。
//!
//! 本模块提供 `Worker` 结构体，用于插件二进制文件的 main 函数。
//!
//! ## 使用方式
//!
//! 插件开发者在 `main()` 中：
//! 1. 创建 `CapabilityRouter` 并注册能力处理器
//! 2. 调用 `Worker::from_stdio()` 创建 worker
//! 3. 调用 `worker.run()` 进入事件循环
//!
//! ## 生命周期
//!
//! `run()` 会阻塞直到宿主关闭连接（通过 `Peer::wait_closed()`）。
//! 在此期间，worker 持续接收宿主的调用请求并返回结果。

use std::sync::Arc;

use astrcode_core::Result;
use astrcode_protocol::plugin::{InitializeMessage, PeerDescriptor};

use crate::transport::StdioTransport;
use crate::{CapabilityRouter, Peer};

/// 进程内插件 Worker—— 通过 stdio 与宿主进程通信。
///
/// 通常用于子进程模式下，插件二进制通过 `Worker::from_stdio()` 创建连接，
/// 然后进入 `run()` 循环直到连接关闭。
///
/// # 示例
///
/// ```ignore
/// let mut router = CapabilityRouter::default();
/// router.register(MyHandler)?;
///
/// let worker = Worker::from_stdio(
///     PeerDescriptor { /* ... */ },
///     router,
///     None,
/// );
/// worker.run().await?;
/// ```
pub struct Worker {
    peer: Peer,
}

impl Worker {
    /// 从标准输入输出创建 Worker。
    ///
    /// # 参数
    ///
    /// * `local_peer` - 本插件的描述信息（ID、名称、版本等）
    /// * `router` - 能力路由器，包含所有已注册的能力处理器
    /// * `local_initialize` - 可选的自定义初始化消息；为 `None` 时使用默认值
    ///
    /// # 注意
    ///
    /// 此方法会自动将 router 中已注册的能力包含在初始化消息中，
    /// 无需手动指定。
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

    /// 进入事件循环，持续处理宿主的调用请求。
    ///
    /// 此方法会阻塞直到宿主关闭连接。通常作为插件 `main()` 的最后一个调用。
    pub async fn run(&self) -> Result<()> {
        self.peer.wait_closed().await;
        Ok(())
    }
}
