//! 适配层转发：复用 core 中定义的 CollaborationExecutor 端口。
//!
//! Why: 执行契约应由 core 统一定义，adapter-tools 仅消费该端口并暴露工具实现。

pub use astrcode_core::CollaborationExecutor;
