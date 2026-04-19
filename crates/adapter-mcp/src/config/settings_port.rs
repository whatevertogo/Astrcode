//! # MCP Settings 接口层
//!
//! 审批 DTO 与持久化端口已下沉到 `astrcode-core`，adapter-mcp 仅做重导出。

pub use astrcode_core::{McpApprovalData, McpApprovalStatus, McpSettingsStore};
