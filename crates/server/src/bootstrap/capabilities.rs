//! # 能力装配
//!
//! 本模块把 server 运行时的能力面拆成两层：
//! - 稳定本地能力（stable local capabilities）：core builtin tools + agent tools
//! - 动态外部能力（dynamic external capabilities）：MCP + plugin
//!
//! `CapabilitySurfaceSync` 负责在外部能力变化时重建整份 surface，
//! 但始终保留稳定本地能力不被刷掉。

use std::{path::Path, sync::Arc};

use astrcode_adapter_skills::{SkillCatalog, SkillSpec, load_builtin_skills};
use astrcode_adapter_tools::{
    agent_tools::{CloseAgentTool, ObserveAgentTool, SendAgentTool, SpawnAgentTool},
    builtin_tools::{
        apply_patch::ApplyPatchTool,
        edit_file::EditFileTool,
        enter_plan_mode::EnterPlanModeTool,
        exit_plan_mode::ExitPlanModeTool,
        find_files::FindFilesTool,
        grep::GrepTool,
        list_dir::ListDirTool,
        read_file::ReadFileTool,
        shell::ShellTool,
        skill_tool::SkillTool,
        tool_search::{ToolSearchIndex, ToolSearchTool},
        upsert_session_plan::UpsertSessionPlanTool,
        write_file::WriteFileTool,
    },
};
use astrcode_application::AgentOrchestrationService;

use super::deps::{
    core::{CapabilityInvoker, Result, Tool},
    kernel::{CapabilityRouter, Kernel, ToolCapabilityInvoker},
};

/// 构建稳定本地层中的 core builtin tool invokers。
///
/// 这里的“builtin”是能力来源语义，不等同于“所有稳定能力”。
/// 例如 agent 四工具同样属于稳定本地能力，但不在本函数中构建，
/// 因为它们依赖 `AgentOrchestrationService`，必须在更晚的组合根阶段装配。
pub(crate) fn build_core_tool_invokers(
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
        Arc::new(EnterPlanModeTool),
        Arc::new(ExitPlanModeTool),
        Arc::new(ToolSearchTool::new(tool_search_index)),
        Arc::new(SkillTool::new(skill_catalog)),
        Arc::new(UpsertSessionPlanTool),
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
pub(crate) fn build_skill_catalog(
    home_dir: &Path,
    mut external_base_skills: Vec<SkillSpec>,
) -> Arc<SkillCatalog> {
    let mut base_skills = load_builtin_skills();
    base_skills.append(&mut external_base_skills);
    Arc::new(SkillCatalog::new_with_home_dir(base_skills, home_dir))
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
    stable_local_invokers: Vec<Arc<dyn CapabilityInvoker>>,
    router: CapabilityRouter,
    kernel: Arc<Kernel>,
    tool_search_index: Arc<ToolSearchIndex>,
}

impl CapabilitySurfaceSync {
    pub(crate) fn new(
        kernel: Arc<Kernel>,
        stable_local_invokers: Vec<Arc<dyn CapabilityInvoker>>,
        tool_search_index: Arc<ToolSearchIndex>,
    ) -> Self {
        Self {
            router: kernel.gateway().capabilities().clone(),
            kernel,
            stable_local_invokers,
            tool_search_index,
        }
    }

    /// 用 MCP + plugin 的外部调用器替换整份 surface。
    ///
    /// 稳定本地调用器（core builtin + agent）始终保留，MCP 和 plugin 由外部传入。
    /// 整份 surface 一次性替换，不做半刷新。
    /// 同时刷新 ToolSearchIndex 使其与 router 保持同步。
    pub(crate) fn apply_external_invokers(
        &self,
        external_invokers: Vec<Arc<dyn CapabilityInvoker>>,
    ) -> Result<()> {
        let mut invokers = self.stable_local_invokers.clone();
        invokers.extend(external_invokers);
        self.router.replace_invokers(invokers.clone())?;
        self.kernel
            .surface()
            .replace_capabilities(&invokers, self.kernel.events());
        let external_specs = invokers
            .iter()
            .skip(self.stable_local_invokers.len())
            .map(|invoker| invoker.capability_spec())
            .collect();
        self.tool_search_index.replace_from_specs(external_specs);
        Ok(())
    }

    pub(crate) fn current_capabilities(&self) -> Vec<super::deps::core::CapabilitySpec> {
        self.kernel.surface().snapshot().capability_specs
    }
}

/// 构建 agent 协作工具（spawn / send / close / observe）的 capability invoker。
///
/// 因为 agent_service 依赖 kernel 和 session_runtime，
/// 而 kernel 的 capability surface 又需要包含 agent 工具，
/// 所以 agent 工具的注册必须在 kernel + session_runtime 构建之后单独完成。
pub(crate) fn build_agent_tool_invokers(
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

/// 合并稳定本地能力层。
///
/// 为什么显式拆成 helper：
/// - `core builtin` 和 `agent tools` 都属于“稳定本地能力”，只是装配时机不同
/// - 组合根里直接 `extend` 很容易把“来源”和“生命周期”两个维度混在一起
pub(crate) fn build_stable_local_invokers(
    core_tool_invokers: Vec<Arc<dyn CapabilityInvoker>>,
    agent_tool_invokers: Vec<Arc<dyn CapabilityInvoker>>,
) -> Vec<Arc<dyn CapabilityInvoker>> {
    let mut stable_local_invokers = core_tool_invokers;
    stable_local_invokers.extend(agent_tool_invokers);
    stable_local_invokers
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use astrcode_adapter_tools::builtin_tools::tool_search::ToolSearchIndex;
    use async_trait::async_trait;
    use serde_json::{Value, json};

    use super::{CapabilitySurfaceSync, build_stable_local_invokers};
    use crate::bootstrap::{
        capabilities::sync_external_tool_search_index,
        deps::{
            core::{
                AstrError, CapabilityInvoker, CapabilityKind, CapabilitySpec,
                CapabilitySpecBuildError, LlmEventSink, LlmOutput, LlmProvider, LlmRequest,
                ModelLimits, PromptBuildOutput, PromptBuildRequest, PromptProvider,
                ResourceProvider, ResourceReadResult, ResourceRequestContext, Result, Tool,
                ToolContext, ToolDefinition, ToolExecutionResult,
            },
            kernel::{CapabilityRouter, Kernel, ToolCapabilityInvoker},
        },
    };

    #[derive(Debug)]
    struct StaticTool {
        name: &'static str,
        tags: &'static [&'static str],
    }

    #[async_trait]
    impl Tool for StaticTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: self.name.to_string(),
                description: format!("tool {}", self.name),
                parameters: json!({"type": "object"}),
            }
        }

        fn capability_spec(&self) -> std::result::Result<CapabilitySpec, CapabilitySpecBuildError> {
            CapabilitySpec::builder(self.name, CapabilityKind::Tool)
                .description(format!("tool {}", self.name))
                .schema(json!({"type": "object"}), json!({"type": "string"}))
                .tags(self.tags.iter().copied())
                .build()
        }

        async fn execute(
            &self,
            tool_call_id: String,
            _input: Value,
            _ctx: &ToolContext,
        ) -> Result<ToolExecutionResult> {
            Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: self.name.to_string(),
                ok: true,
                output: String::new(),
                child_ref: None,
                error: None,
                metadata: None,
                duration_ms: 0,
                truncated: false,
            })
        }
    }

    #[derive(Debug)]
    struct NoopLlmProvider;

    #[async_trait]
    impl LlmProvider for NoopLlmProvider {
        async fn generate(
            &self,
            _request: LlmRequest,
            _sink: Option<LlmEventSink>,
        ) -> Result<LlmOutput> {
            Err(AstrError::Validation(
                "noop llm provider should not execute in this test".to_string(),
            ))
        }

        fn model_limits(&self) -> ModelLimits {
            ModelLimits {
                context_window: 8192,
                max_output_tokens: 4096,
            }
        }
    }

    #[derive(Debug)]
    struct NoopPromptProvider;

    #[async_trait]
    impl PromptProvider for NoopPromptProvider {
        async fn build_prompt(&self, _request: PromptBuildRequest) -> Result<PromptBuildOutput> {
            Ok(PromptBuildOutput {
                system_prompt: "noop".to_string(),
                system_prompt_blocks: Vec::new(),
                cache_metrics: Default::default(),
                metadata: Value::Null,
            })
        }
    }

    #[derive(Debug)]
    struct NoopResourceProvider;

    #[async_trait]
    impl ResourceProvider for NoopResourceProvider {
        async fn read_resource(
            &self,
            _uri: &str,
            _context: &ResourceRequestContext,
        ) -> Result<ResourceReadResult> {
            Ok(ResourceReadResult {
                uri: "noop://resource".to_string(),
                content: Value::Null,
                metadata: Value::Null,
            })
        }
    }

    fn invoker(name: &'static str, tags: &'static [&'static str]) -> Arc<dyn CapabilityInvoker> {
        Arc::new(
            ToolCapabilityInvoker::new(Arc::new(StaticTool { name, tags }))
                .expect("static tool should build"),
        ) as Arc<dyn CapabilityInvoker>
    }

    fn test_kernel(builtin_invokers: &[Arc<dyn CapabilityInvoker>]) -> Arc<Kernel> {
        let mut builder = CapabilityRouter::builder();
        for invoker in builtin_invokers {
            builder = builder.register_invoker(Arc::clone(invoker));
        }
        let router = builder.build().expect("router should build");
        Arc::new(
            Kernel::builder()
                .with_capabilities(router)
                .with_llm_provider(Arc::new(NoopLlmProvider))
                .with_prompt_provider(Arc::new(NoopPromptProvider))
                .with_resource_provider(Arc::new(NoopResourceProvider))
                .build()
                .expect("kernel should build"),
        )
    }

    #[test]
    fn apply_external_invokers_keeps_previous_surface_on_failure() {
        let builtin_invoker = invoker("read_file", &["source:builtin"]);
        let core_tool_invokers = vec![builtin_invoker];
        let kernel = test_kernel(&core_tool_invokers);
        let tool_search_index = Arc::new(ToolSearchIndex::new());
        let sync = CapabilitySurfaceSync::new(
            Arc::clone(&kernel),
            core_tool_invokers.clone(),
            Arc::clone(&tool_search_index),
        );

        let previous_external = invoker("mcp__demo__search", &["source:mcp"]);
        sync.apply_external_invokers(vec![Arc::clone(&previous_external)])
            .expect("initial replace should succeed");
        let previous_specs = sync.current_capabilities();
        let previous_search = tool_search_index.search("demo", 10);
        assert_eq!(previous_search.len(), 1);

        let error = sync.apply_external_invokers(vec![
            invoker("dup_tool", &["source:mcp"]),
            invoker("dup_tool", &["source:plugin"]),
        ]);
        assert!(
            error.is_err(),
            "duplicate capability should fail replacement"
        );

        let current_specs = sync.current_capabilities();
        assert_eq!(current_specs, previous_specs);
        let current_search = tool_search_index.search("demo", 10);
        assert_eq!(current_search, previous_search);
    }

    #[test]
    fn sync_external_tool_search_index_only_indexes_external_sources() {
        let index = ToolSearchIndex::new();
        sync_external_tool_search_index(
            &index,
            &[
                invoker("read_file", &["source:builtin"]),
                invoker("mcp__demo__search", &["source:mcp"]),
                invoker("plugin.search", &["source:plugin"]),
            ],
        );

        let names: Vec<String> = index
            .search("", 10)
            .into_iter()
            .map(|spec| spec.name.to_string())
            .collect();
        assert_eq!(
            names,
            vec!["mcp__demo__search".to_string(), "plugin.search".to_string()]
        );
    }

    #[test]
    fn apply_external_invokers_preserves_stable_internal_tools() {
        let builtin_invoker = invoker("read_file", &["source:builtin"]);
        let agent_invoker = invoker("spawn", &["builtin", "agent"]);
        let stable_local_invokers =
            build_stable_local_invokers(vec![builtin_invoker], vec![agent_invoker]);
        let kernel = test_kernel(&stable_local_invokers);
        let tool_search_index = Arc::new(ToolSearchIndex::new());
        let sync = CapabilitySurfaceSync::new(
            Arc::clone(&kernel),
            stable_local_invokers,
            Arc::clone(&tool_search_index),
        );

        sync.apply_external_invokers(vec![invoker("mcp__demo__search", &["source:mcp"])])
            .expect("replace should succeed");

        let names: Vec<String> = kernel.gateway().capabilities().tool_names();
        assert!(names.iter().any(|name| name == "read_file"));
        assert!(names.iter().any(|name| name == "spawn"));
        assert!(names.iter().any(|name| name == "mcp__demo__search"));
    }
}
