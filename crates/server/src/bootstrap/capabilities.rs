//! # 能力装配
//!
//! 负责把内置工具适配为 `CapabilityRouter`，
//! 并在外部 surface（如 MCP）变化时同步刷新 kernel 能力面。

use std::sync::Arc;

use astrcode_adapter_skills::{SkillCatalog, SkillSpec, load_builtin_skills};
use astrcode_adapter_tools::{
    agent_tools::{CloseAgentTool, ObserveAgentTool, SendAgentTool, SpawnAgentTool},
    builtin_tools::{
        apply_patch::ApplyPatchTool,
        edit_file::EditFileTool,
        find_files::FindFilesTool,
        grep::GrepTool,
        list_dir::ListDirTool,
        read_file::ReadFileTool,
        shell::ShellTool,
        skill_tool::SkillTool,
        tool_search::{ToolSearchIndex, ToolSearchTool},
        write_file::WriteFileTool,
    },
};
use astrcode_application::AgentOrchestrationService;
use astrcode_core::{CapabilityInvoker, Result, Tool};
use astrcode_kernel::{CapabilityRouter, Kernel, ToolCapabilityInvoker};

pub(crate) fn build_builtin_capability_invokers(
    tool_search_index: Arc<ToolSearchIndex>,
    skill_catalog: Arc<SkillCatalog>,
) -> Result<Vec<Arc<dyn CapabilityInvoker>>> {
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(ReadFileTool),
        Arc::new(WriteFileTool),
        Arc::new(EditFileTool),
        Arc::new(ApplyPatchTool),
        Arc::new(ListDirTool),
        Arc::new(FindFilesTool),
        Arc::new(GrepTool),
        Arc::new(ShellTool),
        Arc::new(ToolSearchTool::new(tool_search_index)),
        Arc::new(SkillTool::new(skill_catalog)),
    ];

    let invokers = tools
        .into_iter()
        .filter_map(|tool| match ToolCapabilityInvoker::new(tool) {
            Ok(invoker) => Some(Arc::new(invoker) as Arc<dyn CapabilityInvoker>),
            Err(error) => {
                log::error!("注册工具失败: {error}");
                None
            },
        })
        .collect();

    Ok(invokers)
}

/// 创建统一的 SkillCatalog。
///
/// base skills 的顺序必须满足 `builtin < mcp < plugin`，
/// 这样 catalog 才能在后续叠加 user/project 时保持正确优先级。
pub(crate) fn build_skill_catalog(mut external_base_skills: Vec<SkillSpec>) -> Arc<SkillCatalog> {
    let mut base_skills = load_builtin_skills();
    base_skills.append(&mut external_base_skills);
    Arc::new(SkillCatalog::new(base_skills))
}

/// 让 tool_search 索引与当前外部能力事实源保持同步。
///
/// 启动路径和 reload 路径都必须调用这段逻辑，避免前者有能力、
/// 后者才有搜索索引，导致两个事实源出现漂移。
pub(crate) fn sync_external_tool_search_index(
    tool_search_index: &ToolSearchIndex,
    external_invokers: &[Arc<dyn CapabilityInvoker>],
) {
    let external_specs = external_invokers
        .iter()
        .map(|invoker| invoker.capability_spec())
        .collect();
    tool_search_index.replace_from_specs(external_specs);
}

pub(crate) fn build_server_capability_router(
    invokers: Vec<Arc<dyn CapabilityInvoker>>,
) -> Result<CapabilityRouter> {
    let router = CapabilityRouter::empty();
    router.register_invokers(invokers)?;
    Ok(router)
}

#[derive(Clone)]
pub(crate) struct CapabilitySurfaceSync {
    builtin_invokers: Vec<Arc<dyn CapabilityInvoker>>,
    router: CapabilityRouter,
    kernel: Arc<Kernel>,
    tool_search_index: Arc<ToolSearchIndex>,
}

impl CapabilitySurfaceSync {
    pub(crate) fn new(
        kernel: Arc<Kernel>,
        builtin_invokers: Vec<Arc<dyn CapabilityInvoker>>,
        tool_search_index: Arc<ToolSearchIndex>,
    ) -> Self {
        Self {
            router: kernel.gateway().capabilities().clone(),
            kernel,
            builtin_invokers,
            tool_search_index,
        }
    }

    /// 用 MCP + plugin 的外部调用器替换整份 surface。
    ///
    /// builtin 始终保留，MCP 和 plugin 由外部传入。
    /// 整份 surface 一次性替换，不做半刷新。
    /// 同时刷新 ToolSearchIndex 使其与 router 保持同步。
    pub(crate) fn apply_external_invokers(
        &self,
        external_invokers: Vec<Arc<dyn CapabilityInvoker>>,
    ) -> Result<()> {
        let mut invokers = self.builtin_invokers.clone();

        sync_external_tool_search_index(&self.tool_search_index, &external_invokers);

        invokers.extend(external_invokers);
        self.router.replace_invokers(invokers.clone())?;
        self.kernel
            .surface()
            .replace_capabilities(&invokers, self.kernel.events());
        Ok(())
    }
}

/// 构建 agent 四工具（spawn / send / close / observe）的 capability invoker。
///
/// 因为 agent_service 依赖 kernel 和 session_runtime，
/// 而 kernel 的 capability surface 又需要包含 agent 工具，
/// 所以 agent 工具的注册必须在 kernel + session_runtime 构建之后单独完成。
pub(crate) fn build_agent_invokers(
    agent_service: Arc<AgentOrchestrationService>,
) -> Result<Vec<Arc<dyn CapabilityInvoker>>> {
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(SpawnAgentTool::new(agent_service.clone())),
        Arc::new(SendAgentTool::new(agent_service.clone())),
        Arc::new(CloseAgentTool::new(agent_service.clone())),
        Arc::new(ObserveAgentTool::new(agent_service)),
    ];
    Ok(tools
        .into_iter()
        .filter_map(|tool| match ToolCapabilityInvoker::new(tool) {
            Ok(invoker) => Some(Arc::new(invoker) as Arc<dyn CapabilityInvoker>),
            Err(error) => {
                log::error!("注册 Agent 工具失败: {error}");
                None
            },
        })
        .collect())
}
