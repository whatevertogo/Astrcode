//! # 会话管理器
//!
//! 定义会话管理的抽象接口和文件系统实现。

use crate::{event::EventLog, session::DeleteProjectResult, session::SessionMeta, Result};

/// 会话管理器抽象
///
/// 定义了会话的 CRUD 操作。具体实现可以是文件系统、数据库等。
pub trait SessionManager: Send + Sync {
    /// 列出所有会话 ID
    fn list_sessions(&self) -> Result<Vec<String>>;
    /// 列出所有会话的元数据（包含标题、状态等）
    fn list_sessions_with_meta(&self) -> Result<Vec<SessionMeta>>;
    /// 删除单个会话
    fn delete_session(&self, session_id: &str) -> Result<()>;
    /// 删除指定工作目录下的所有会话（用于项目删除）
    fn delete_sessions_by_working_dir(&self, working_dir: &str) -> Result<DeleteProjectResult>;
    /// 创建新的事件日志
    fn create_event_log(&self, session_id: &str) -> Result<EventLog>;
    /// 打开现有的事件日志
    fn open_event_log(&self, session_id: &str) -> Result<EventLog>;
}

/// 文件系统会话仓储
///
/// 基于 `~/.astrcode/sessions/` 目录的会话管理实现。
#[derive(Debug, Default, Clone, Copy)]
pub struct FileSystemSessionRepository;

impl SessionManager for FileSystemSessionRepository {
    fn list_sessions(&self) -> Result<Vec<String>> {
        EventLog::list_sessions()
    }

    fn list_sessions_with_meta(&self) -> Result<Vec<SessionMeta>> {
        EventLog::list_sessions_with_meta()
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        EventLog::delete_session(session_id)
    }

    fn delete_sessions_by_working_dir(&self, working_dir: &str) -> Result<DeleteProjectResult> {
        EventLog::delete_sessions_by_working_dir(working_dir)
    }

    fn create_event_log(&self, session_id: &str) -> Result<EventLog> {
        EventLog::create(session_id)
    }

    fn open_event_log(&self, session_id: &str) -> Result<EventLog> {
        EventLog::open(session_id)
    }
}
