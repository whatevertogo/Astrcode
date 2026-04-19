//! MCP 审批共享模型。

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpApprovalData {
    pub server_signature: String,
    pub status: McpApprovalStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_by: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpApprovalStatus {
    Pending,
    Approved,
    Rejected,
}
