//! 会话 façade 的内部实现按用例拆分：
//! - `create`：创建与列出 session
//! - `load`：加载、快照、重放前置读取
//! - `delete`：删除 session / project

mod create;
mod delete;
mod load;

pub(crate) use load::load_events;
