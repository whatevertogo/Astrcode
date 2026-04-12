//! # MCP Settings 接口层
//!
//! 定义 `McpSettingsStore` trait 和审批数据 DTO。
//! 这是纯接口层，不含任何 IO 实现——具体实现由 runtime 在 bootstrap 时注入。

use serde::{Deserialize, Serialize};

/// MCP 审批数据——记录单个服务器的审批状态。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpApprovalData {
    /// 服务器签名（用于唯一标识，如 command:args 或 URL）。
    pub server_signature: String,
    /// 审批状态。
    pub status: McpApprovalStatus,
    /// 审批时间（ISO 8601 格式）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<String>,
    /// 审批来源（用户标识或 "auto"）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_by: Option<String>,
}

/// MCP 服务器审批状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpApprovalStatus {
    /// 等待用户审批。
    Pending,
    /// 用户已批准。
    Approved,
    /// 用户已拒绝。
    Rejected,
}

/// MCP settings 持久化接口。
///
/// runtime-mcp 通过此 trait 读写审批数据，
/// 不直接读写 settings 文件。
/// 具体实现由 runtime 在 bootstrap 时注入。
pub trait McpSettingsStore: Send + Sync {
    /// 加载指定项目的所有审批数据。
    fn load_approvals(
        &self,
        project_path: &str,
    ) -> std::result::Result<Vec<McpApprovalData>, String>;

    /// 保存审批数据。
    fn save_approval(
        &self,
        project_path: &str,
        data: &McpApprovalData,
    ) -> std::result::Result<(), String>;
}
