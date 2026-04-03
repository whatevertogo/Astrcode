//! 插件传输层。
//!
//! 本模块定义了插件宿主与插件进程之间的传输抽象。
//!
//! ## 架构
//!
//! `Transport` trait 定义了最基本的发送/接收接口，
//! 当前唯一的实现是 `StdioTransport`，通过标准输入输出进行 JSON-RPC 通信。
//!
//! ## 扩展性
//!
//! 未来可以添加其他传输实现（如 TCP、Unix socket 等），
//! 只需实现 `Transport` trait 即可与现有的 `Peer` 兼容。

mod stdio;

pub use astrcode_protocol::transport::Transport;
pub use stdio::StdioTransport;
