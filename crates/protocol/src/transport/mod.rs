//! 传输层模块
//!
//! 定义跨进程通信的传输抽象。当前实现基于 stdio 的 JSON-RPC 消息传输，
//! 通过 `Transport` trait 抽象 send/recv 操作，使上层协议不依赖具体传输方式。

mod traits;

pub use traits::Transport;
