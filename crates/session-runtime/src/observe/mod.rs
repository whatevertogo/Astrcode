//! 会话观测与事件过滤。
//!
//! 从 `runtime/service/agent/` 和 `runtime/service/session/` 迁入
//! observe/view 相关的类型定义。实际执行逻辑在 Phase 10 组合根接线。
//!
//! 边界约束：
//! - `observe` 只承载 replay/live 订阅语义、scope/filter 与状态来源
//! - 同步快照投影算法统一留在 `query`

use astrcode_core::SubRunHandle;

use crate::state::SessionSnapshot;

/// 会话观测快照。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionObserveSnapshot {
    pub state: SessionSnapshot,
}

/// 事件过滤范围：按谱系层级过滤 sub-run 事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubRunEventScope {
    /// 仅指定 sub-run 自身的事件。
    SelfOnly,
    /// 指定 sub-run 的直接子节点事件。
    DirectChildren,
    /// 指定 sub-run 的整棵子树事件。
    Subtree,
}

/// 事件过滤参数。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEventFilterSpec {
    pub target_sub_run_id: String,
    pub scope: SubRunEventScope,
}

/// Sub-run 状态来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubRunStatusSource {
    Live,
    Durable,
}

/// Sub-run 状态快照。
#[derive(Debug, Clone)]
pub struct SubRunStatusSnapshot {
    pub handle: SubRunHandle,
    pub tool_call_id: Option<String>,
    pub source: SubRunStatusSource,
    pub result: Option<astrcode_core::SubRunResult>,
    pub step_count: Option<u32>,
    pub estimated_tokens: Option<u64>,
}
