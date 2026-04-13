//! 适配层转发：复用 core 中定义的 SubAgentExecutor 端口。
//!
//! Why: 执行契约属于业务边界，不属于 adapter；此处仅保留导出兼容路径。

pub use astrcode_core::SubAgentExecutor;
