//! 传输 trait 定义
//!
//! 定义跨进程通信的基本接口，所有具体传输实现（如 stdio、TCP 等）
//! 都必须实现此 trait。

use async_trait::async_trait;

/// 跨进程通信的传输抽象。
///
/// 定义了发送和接收消息的基本接口。实现此 trait 的类型可以用于插件与 host 之间的
/// JSON-RPC 消息传输。
///
/// ## 线程安全
///
/// `Send + Sync` 约束确保传输实例可以在线程间安全共享，
/// 支持并发发送和接收操作。
///
/// ## 错误处理
///
/// `send` 和 `recv` 都返回 `Result<_, String>`，错误信息为人类可读的描述。
/// `recv` 返回 `Option<String>`，`None` 表示传输通道已关闭（EOF）。
#[async_trait]
pub trait Transport: Send + Sync {
    /// 发送一条消息。
    ///
    /// `payload` 为已序列化的 JSON 字符串。实现负责将其写入底层传输通道。
    async fn send(&self, payload: &str) -> Result<(), String>;

    /// 接收一条消息。
    ///
    /// 返回 `Some(String)` 表示收到消息，`None` 表示传输通道已关闭。
    /// 此方法应该是阻塞的，直到有消息可用或通道关闭。
    async fn recv(&self) -> Result<Option<String>, String>;
}
