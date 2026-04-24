//! mode 切换工具共享辅助。
//!
//! `enterPlanMode` 与 `exitPlanMode` 都需要发出相同的 `ModeChanged` 事件，
//! 这里集中实现，避免工具层对同一条领域事件各自维护一份写法。

use astrcode_core::{AgentEventContext, AstrError, Result, StorageEvent, StorageEventPayload};
use astrcode_governance_contract::ModeId;
use astrcode_tool_contract::ToolContext;
use chrono::Utc;

pub async fn emit_mode_changed(
    ctx: &ToolContext,
    tool_name: &'static str,
    from: ModeId,
    to: ModeId,
) -> Result<()> {
    let Some(event_sink) = ctx.event_sink() else {
        return Err(AstrError::Internal(format!(
            "{tool_name} requires an attached tool event sink"
        )));
    };
    event_sink
        .emit(StorageEvent {
            turn_id: None,
            agent: AgentEventContext::default(),
            payload: StorageEventPayload::ModeChanged {
                from: from.into(),
                to: to.into(),
                timestamp: Utc::now(),
            },
        })
        .await
}
