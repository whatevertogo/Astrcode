//! # MCP 配置管理
//!
//! 负责从多个作用域加载 MCP 服务器配置，支持去重和环境变量展开。

pub mod approval;
pub mod loader;
pub mod policy;
pub mod settings_port;
mod types;

pub use approval::McpApprovalManager;
pub use loader::McpConfigManager;
pub use policy::McpPolicyFilter;
pub use settings_port::{McpApprovalData, McpApprovalStatus, McpSettingsStore};
pub use types::*;
