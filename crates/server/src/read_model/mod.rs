//! server read-model 层。
//!
//! 这里放 conversation/terminal 投影、replay 与 snapshot 构造，HTTP handler 只负责传输。

pub(crate) mod conversation;
pub(crate) mod terminal;
pub(crate) mod view_projection;
