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

use astrcode_core::{CapabilityInvoker, Result};
use astrcode_runtime_agent_tool::{
    CloseAgentTool, CollaborationExecutor, DeliverToParentTool, ResumeAgentTool, SendAgentTool,
    SpawnAgentTool, SubAgentExecutor, WaitAgentTool,
};
use astrcode_runtime_registry::ToolCapabilityInvoker;
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
    collaboration_executor: Arc<dyn CollaborationExecutor>,
) -> Result<Vec<Arc<dyn CapabilityInvoker>>> {
    vec![
        ToolCapabilityInvoker::boxed(Box::new(SkillTool::new(skill_catalog))),
        // 注意 SpawnAgentTool 依赖 SubAgentExecutor，因此放在 SkillTool 之后注册，确保
        // subagent_executor 已经准备好 子agent工具
        ToolCapabilityInvoker::boxed(Box::new(SpawnAgentTool::new(subagent_executor))),
        // 协作工具族：通过 CollaborationExecutor 统一委托
        ToolCapabilityInvoker::boxed(Box::new(SendAgentTool::new(collaboration_executor.clone()))),
        ToolCapabilityInvoker::boxed(Box::new(WaitAgentTool::new(collaboration_executor.clone()))),
        ToolCapabilityInvoker::boxed(Box::new(CloseAgentTool::new(
            collaboration_executor.clone(),
        ))),
        ToolCapabilityInvoker::boxed(Box::new(ResumeAgentTool::new(
            collaboration_executor.clone(),
        ))),
        ToolCapabilityInvoker::boxed(Box::new(DeliverToParentTool::new(collaboration_executor))),
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
