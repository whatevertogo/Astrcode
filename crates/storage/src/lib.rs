//! # Astrcode 存储层
//!
//! 提供基于本地文件系统的持久化实现，为 Astrcode 运行时提供会话事件存储、
//! 会话元数据管理、以及基于文件锁的并发写入保护。
//!
//! ## 存储模型
//!
//! 所有会话数据以 JSONL（JSON Lines）格式追加写入，路径约定为：
//! `~/.astrcode/projects/<project>/sessions/<session-id>/session-<session-id>.jsonl`
//!
//! 每个事件以 `StoredEvent { storage_seq, event }` 结构持久化，其中
//! `storage_seq` 是单调递增的序列号，由会话 writer 独占分配，保证全局有序。
//!
//! ## 核心组件
//!
//! - [`session::EventLog`] — JSONL 事件日志的创建、打开、追加与回放
//! - [`session::EventLogIterator`] — 逐行流式读取会话事件
//! - [`session::FileSystemSessionRepository`] — 实现 `SessionManager` trait 的门面，
//!   组合事件日志、迭代器与文件锁，提供统一的会话管理接口
//!
//! ## 并发安全
//!
//! 会话写入通过 `active-turn.lock` 文件锁实现互斥，防止多进程同时写入同一会话。
//! 锁持有者同时在 `active-turn.json` 中写入元数据（turn_id、owner_pid、acquired_at），
//! 以便竞争者获取当前持有者信息并做出相应处理。

pub mod session;

use astrcode_core::store::{StoreError, StoreResult};

/// 存储层内部使用的 Result 别名，统一错误类型为 [`StoreError`]。
pub(crate) type Result<T> = StoreResult<T>;

pub(crate) struct AstrError;

impl AstrError {
    pub(crate) fn io(context: impl Into<String>, source: std::io::Error) -> StoreError {
        StoreError::Io {
            context: context.into(),
            source,
        }
    }

    pub(crate) fn parse(context: impl Into<String>, source: serde_json::Error) -> StoreError {
        StoreError::Parse {
            context: context.into(),
            source,
        }
    }
}

pub(crate) fn internal_io_error(context: impl Into<String>) -> StoreError {
    StoreError::Io {
        context: context.into(),
        source: std::io::Error::other("storage invariant violation"),
    }
}
