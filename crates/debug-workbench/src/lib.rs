//! Debug Workbench 后端读模型。
//!
//! 该 crate 只承载 debug 查询与聚合逻辑：
//! - runtime overview
//! - 最近时间窗口趋势
//! - session trace
//! - session agent tree
//!
//! HTTP、Tauri 和前端都不在这里实现，server 仍然是唯一组合根。

mod models;
mod service;

pub use models::{
    DebugAgentNodeKind, RuntimeDebugOverview, RuntimeDebugTimeline, RuntimeDebugTimelineSample,
    SessionDebugAgentNode, SessionDebugAgents, SessionDebugTrace, SessionDebugTraceItem,
    SessionDebugTraceItemKind,
};
pub use service::DebugWorkbenchService;
