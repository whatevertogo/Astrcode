//! # 内置能力 (Built-in Capabilities)
//!
//! 注册所有内置工具的调用器（Invoker），包括：
//! - `Skill` - Skill 加载工具
//! - `shell` - 命令执行
//! - `listDir` / `readFile` / `writeFile` / `editFile` - 文件操作
//! - `findFiles` / `grep` - 文件搜索
//!
//! ## 注册顺序
//!
//! 注册顺序不影响功能，但为保持一致性，按以下顺序排列：
//! 1. Skill 工具（特殊的元工具）
//! 2. Shell 工具（最高频使用）
//! 3. 文件操作工具
//! 4. 搜索工具

use std::sync::Arc;

use astrcode_core::{CapabilityInvoker, Result, ToolCapabilityInvoker};

use crate::skill_tool::SkillTool;
use astrcode_runtime_skill_loader::SkillCatalog;

/// 构建所有内置能力的调用器列表。
///
/// 返回的调用器包括 Skill 元工具、Shell 工具、文件操作工具和搜索工具。
/// 这些调用器会被注册到 `CapabilityRouter` 中，供 Agent 循环调用。
///
/// ## 注册顺序
///
/// 注册顺序不影响功能，但为保持一致性，按以下顺序排列：
/// 1. Skill 工具（特殊的元工具）
/// 2. Shell 工具（最高频使用）
/// 3. 文件操作工具
/// 4. 搜索工具
pub(crate) fn built_in_capability_invokers(
    skill_catalog: Arc<SkillCatalog>,
) -> Result<Vec<Arc<dyn CapabilityInvoker>>> {
    vec![
        ToolCapabilityInvoker::boxed(Box::new(SkillTool::new(skill_catalog))),
        ToolCapabilityInvoker::boxed(Box::new(astrcode_tools::tools::shell::ShellTool)),
        ToolCapabilityInvoker::boxed(Box::new(astrcode_tools::tools::list_dir::ListDirTool)),
        ToolCapabilityInvoker::boxed(Box::new(astrcode_tools::tools::read_file::ReadFileTool)),
        ToolCapabilityInvoker::boxed(Box::new(astrcode_tools::tools::write_file::WriteFileTool)),
        ToolCapabilityInvoker::boxed(Box::new(astrcode_tools::tools::edit_file::EditFileTool)),
        ToolCapabilityInvoker::boxed(Box::new(astrcode_tools::tools::find_files::FindFilesTool)),
        ToolCapabilityInvoker::boxed(Box::new(astrcode_tools::tools::grep::GrepTool)),
    ]
    .into_iter()
    .collect()
}
