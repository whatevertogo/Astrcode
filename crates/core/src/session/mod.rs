//! # 会话管理
//!
//! 负责会话的持久化管理、列表查询和删除操作。

mod manager;
mod types;
mod writer;

pub use manager::{FileSystemSessionRepository, SessionManager};
pub use types::{DeleteProjectResult, SessionEventRecord, SessionMessage, SessionMeta};
pub use writer::SessionWriter;
