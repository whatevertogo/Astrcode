//! # 内置能力 (Built-in Capabilities)
//!
//! 注册所有内置工具的调用器（Invoker），包括：
//! - `Skill` - Skill 加载工具
//! - `shell` - 命令执行
//! - `listDir` / `readFile` / `writeFile` / `editFile` / `apply_patch` - 文件操作
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
use astrcode_runtime_agent_tool::{RunAgentTool, SubAgentExecutor};
use astrcode_runtime_skill_loader::SkillCatalog;

use crate::skill_tool::SkillTool;

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
    subagent_executor: Arc<dyn SubAgentExecutor>,
) -> Result<Vec<Arc<dyn CapabilityInvoker>>> {
    vec![
        ToolCapabilityInvoker::boxed(Box::new(SkillTool::new(skill_catalog))),
        ToolCapabilityInvoker::boxed(Box::new(RunAgentTool::new(subagent_executor))),
        ToolCapabilityInvoker::boxed(Box::new(
            astrcode_runtime_tool_loader::builtin_tools::shell::ShellTool,
        )),
        ToolCapabilityInvoker::boxed(Box::new(
            astrcode_runtime_tool_loader::builtin_tools::list_dir::ListDirTool,
        )),
        ToolCapabilityInvoker::boxed(Box::new(
            astrcode_runtime_tool_loader::builtin_tools::read_file::ReadFileTool,
        )),
        ToolCapabilityInvoker::boxed(Box::new(
            astrcode_runtime_tool_loader::builtin_tools::write_file::WriteFileTool,
        )),
        ToolCapabilityInvoker::boxed(Box::new(
            astrcode_runtime_tool_loader::builtin_tools::edit_file::EditFileTool,
        )),
        ToolCapabilityInvoker::boxed(Box::new(
            astrcode_runtime_tool_loader::builtin_tools::apply_patch::ApplyPatchTool,
        )),
        ToolCapabilityInvoker::boxed(Box::new(
            astrcode_runtime_tool_loader::builtin_tools::find_files::FindFilesTool,
        )),
        ToolCapabilityInvoker::boxed(Box::new(
            astrcode_runtime_tool_loader::builtin_tools::grep::GrepTool,
        )),
    ]
    .into_iter()
    .collect()
}
