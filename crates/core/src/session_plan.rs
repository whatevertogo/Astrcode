//! session plan 领域模型。
//!
//! application 与 adapter-tools 需要读写同一份 `state.json`，状态结构和内容摘要算法
//! 必须保持单一真相，避免跨 crate 漂移。

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const SESSION_PLAN_DRAFT_APPROVAL_GUARD_MARKER: &str = "[session-plan:draft-approval-guard]";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPlanStatus {
    Draft,
    AwaitingApproval,
    Approved,
    Completed,
    Superseded,
}

impl SessionPlanStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Approved => "approved",
            Self::Completed => "completed",
            Self::Superseded => "superseded",
        }
    }
}

impl fmt::Display for SessionPlanStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPlanState {
    pub active_plan_slug: String,
    pub title: String,
    pub status: SessionPlanStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_plan_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_plan_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
}

pub fn session_plan_content_digest(content: &str) -> String {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in content.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("fnv1a64:{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_plan_digest_is_stable_and_versioned() {
        assert_eq!(
            session_plan_content_digest("plan body"),
            "fnv1a64:58c14a8c5354d2ea"
        );
    }
}
