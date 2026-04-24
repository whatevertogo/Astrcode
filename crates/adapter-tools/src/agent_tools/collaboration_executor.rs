//! 适配层转发：复用 host-session owner bridge 中定义的协作端口。
//!
//! Why: 协作执行契约属于 session owner，adapter-tools 仅消费该端口并暴露工具实现。

pub use astrcode_host_session::CollaborationExecutor;
