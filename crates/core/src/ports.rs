//! 剩余共享配置端口。
//!
//! 运行时、会话和插件 owner 专属端口已经迁入各自 crate。这里仅保留仍被
//! 多个 owner 共享的配置与本地技能目录端口，避免 `core::ports` 继续作为
//! provider/session/plugin 的 mega 入口。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Config, ConfigOverlay, McpApprovalData, Result, SkillSpec};

/// MCP 配置文件作用域。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpConfigFileScope {
    User,
    Project,
    Local,
}

/// Skill 查询端口。
pub trait SkillCatalog: Send + Sync {
    fn resolve_for_working_dir(&self, working_dir: &str) -> Vec<SkillSpec>;
}

/// MCP settings 持久化端口。
pub trait McpSettingsStore: Send + Sync {
    fn load_approvals(
        &self,
        project_path: &str,
    ) -> std::result::Result<Vec<McpApprovalData>, String>;

    fn save_approval(
        &self,
        project_path: &str,
        data: &McpApprovalData,
    ) -> std::result::Result<(), String>;

    fn clear_approvals(&self, project_path: &str) -> std::result::Result<(), String>;
}

/// 配置存储端口。
///
/// 将配置文件 IO 从 application 层剥离，由 adapter 层实现。
pub trait ConfigStore: Send + Sync {
    /// 从磁盘加载配置（文件不存在时创建默认配置）。
    fn load(&self) -> Result<Config>;
    /// 保存配置到磁盘（原子写入）。
    fn save(&self, config: &Config) -> Result<()>;
    /// 返回配置文件路径。
    fn path(&self) -> PathBuf;
    /// 加载项目 overlay（文件存在时）。
    fn load_overlay(&self, working_dir: &std::path::Path) -> Result<Option<ConfigOverlay>>;
    /// 保存项目 overlay；当值为空时允许实现删除文件。
    fn save_overlay(&self, working_dir: &std::path::Path, overlay: &ConfigOverlay) -> Result<()>;
    /// 读取指定作用域的独立 MCP 原始配置。
    fn load_mcp(
        &self,
        scope: McpConfigFileScope,
        working_dir: Option<&std::path::Path>,
    ) -> Result<Option<Value>>;
    /// 保存指定作用域的独立 MCP 原始配置；当值为空时允许实现删除文件。
    fn save_mcp(
        &self,
        scope: McpConfigFileScope,
        working_dir: Option<&std::path::Path>,
        mcp: Option<&Value>,
    ) -> Result<()>;
}
