//! 会话 HTTP 路由按交互类型拆分：
//! - `query`：只读查询接口
//! - `mutation`：写操作与状态改变
//! - `stream`：SSE / 订阅类接口

mod mutation;
mod query;
mod stream;

pub(crate) use mutation::{
    compact_session, create_session, delete_project, delete_session, interrupt_session,
    submit_prompt,
};
pub(crate) use query::{list_sessions, session_history, session_messages};
pub(crate) use stream::{session_catalog_events, session_events};
