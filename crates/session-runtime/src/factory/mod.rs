//! 单 session 执行构造辅助。
//!
//! Why: `session-runtime` 需要少量执行构造胶水，
//! 但这些胶水不应该散落在 `turn` 或 `application` 里。

use astrcode_core::{LlmMessage, Result, SessionTurnLease};

use crate::state::SessionState;

#[derive(Debug)]
pub struct NoopSessionTurnLease;

impl SessionTurnLease for NoopSessionTurnLease {}

pub fn prepare_turn_messages(session: &SessionState) -> Result<Vec<LlmMessage>> {
    Ok(session.snapshot_projected_state()?.messages)
}
