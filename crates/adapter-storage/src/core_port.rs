//! `adapter-storage` 面向组合根暴露的 `EventStore` 入口。
//!
//! `FileSystemSessionRepository` 现在直接实现 `core::ports::EventStore`，
//! 这里仅保留兼容名称，避免组合根继续感知旧的 `SessionManager` 桥接层。

pub use crate::session::FileSystemSessionRepository as FsEventStore;
