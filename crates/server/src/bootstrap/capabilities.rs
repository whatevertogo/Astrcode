//! # 能力装配
//!
//! 本模块把 server 运行时的能力面拆成两层：
//! - 稳定本地能力（stable local capabilities）：core builtin tools + agent tools
//! - 动态外部能力（dynamic external capabilities）：MCP + plugin
//!
//! `CapabilitySurfaceSync` 负责在外部能力变化时重建整份 surface，
//! 但始终保留稳定本地能力不被刷掉。

use std::{
    path::Path,
    sync::{Arc, RwLock},
};

use astrcode_adapter_skills::{LayeredSkillCatalog, load_builtin_skills};
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
        task_write::TaskWriteTool,
        tool_search::{ToolSearchIndex, ToolSearchTool},
        upsert_session_plan::UpsertSessionPlanTool,
        write_file::WriteFileTool,
    },
};
use astrcode_core::{CapabilitySpec, SkillCatalog, SkillSpec};
use astrcode_host_session::{CollaborationExecutor, SubAgentExecutor};
use astrcode_plugin_host::{ResourceCatalog, build_skill_catalog_base};
use astrcode_tool_contract::Tool;

use super::deps::core::{CapabilityInvoker, Result};
use crate::{
    session_runtime_owner_bridge::ServerCapabilitySurfacePort,
    tool_capability_invoker::ToolCapabilityInvoker,
};

/// 构建稳定本地层中的 core builtin tool invokers。
///
/// 这里的“builtin”是能力来源语义，不等同于“所有稳定能力”。
/// 例如 agent 四工具同样属于稳定本地能力，但不在本函数中构建，
/// 因为它们依赖协作执行 trait object，必须在更晚的组合根阶段装配。
pub(crate) fn build_core_tool_invokers(
    tool_search_index: Arc<ToolSearchIndex>,
    skill_catalog: Arc<dyn SkillCatalog>,
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
        Arc::new(TaskWriteTool),
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
    external_base_skills: Vec<SkillSpec>,
    resource_catalog: &ResourceCatalog,
) -> Arc<LayeredSkillCatalog> {
    let base_build = build_skill_catalog_base(
        load_builtin_skills(),
        external_base_skills,
        resource_catalog,
    );
    Arc::new(LayeredSkillCatalog::new_with_home_dir(
        base_build.base_skills,
        home_dir,
    ))
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

#[derive(Clone)]
pub(crate) struct CapabilitySurfaceSync {
    stable_local_invokers: Vec<Arc<dyn CapabilityInvoker>>,
    capability_surface: Arc<dyn ServerCapabilitySurfacePort>,
    tool_search_index: Arc<ToolSearchIndex>,
    current_capabilities: Arc<RwLock<Vec<CapabilitySpec>>>,
    current_external_invokers: Arc<RwLock<Vec<Arc<dyn CapabilityInvoker>>>>,
}

impl CapabilitySurfaceSync {
    pub(crate) fn new(
        capability_surface: Arc<dyn ServerCapabilitySurfacePort>,
        stable_local_invokers: Vec<Arc<dyn CapabilityInvoker>>,
        tool_search_index: Arc<ToolSearchIndex>,
    ) -> Self {
        Self {
            capability_surface,
            current_capabilities: Arc::new(RwLock::new(
                stable_local_invokers
                    .iter()
                    .map(|invoker| invoker.capability_spec())
                    .collect(),
            )),
            stable_local_invokers,
            tool_search_index,
            current_external_invokers: Arc::new(RwLock::new(Vec::new())),
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
        invokers.extend(external_invokers.clone());
        self.capability_surface
            .replace_capability_invokers(invokers.clone())?;
        let external_specs = invokers
            .iter()
            .skip(self.stable_local_invokers.len())
            .map(|invoker| invoker.capability_spec())
            .collect();
        self.tool_search_index.replace_from_specs(external_specs);
        *self
            .current_capabilities
            .write()
            .expect("capability surface sync current capabilities lock should not be poisoned") =
            invokers
                .iter()
                .map(|invoker| invoker.capability_spec())
                .collect();
        *self
            .current_external_invokers
            .write()
            .expect("capability surface sync external invokers lock should not be poisoned") =
            external_invokers;
        Ok(())
    }

    pub(crate) fn current_capabilities(&self) -> Vec<CapabilitySpec> {
        self.current_capabilities
            .read()
            .expect("capability surface sync current capabilities lock should not be poisoned")
            .clone()
    }

    pub(crate) fn current_external_invokers(&self) -> Vec<Arc<dyn CapabilityInvoker>> {
        self.current_external_invokers
            .read()
            .expect("capability surface sync external invokers lock should not be poisoned")
            .clone()
    }
}

/// 构建 agent 协作工具（spawn / send / close / observe）的 capability invoker。
///
/// 因为 agent_service 依赖 kernel 和 session_runtime，
/// 而 kernel 的 capability surface 又需要包含 agent 工具，
/// 所以 agent 工具的注册必须在 kernel + session_runtime 构建之后单独完成。
pub(crate) fn build_agent_tool_invokers(
    subagent_executor: Arc<dyn SubAgentExecutor>,
    collaboration_executor: Arc<dyn CollaborationExecutor>,
) -> Result<Vec<Arc<dyn CapabilityInvoker>>> {
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(SpawnAgentTool::new(subagent_executor)),
        Arc::new(SendAgentTool::new(Arc::clone(&collaboration_executor))),
        Arc::new(CloseAgentTool::new(Arc::clone(&collaboration_executor))),
        Arc::new(ObserveAgentTool::new(collaboration_executor)),
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
    use std::{
        collections::HashSet,
        sync::{Arc, RwLock},
    };

    use astrcode_adapter_tools::builtin_tools::tool_search::ToolSearchIndex;
    use astrcode_plugin_host::ResourceCatalog;
    use astrcode_tool_contract::{Tool, ToolContext, ToolDefinition, ToolExecutionResult};
    use async_trait::async_trait;
    use serde_json::{Value, json};

    use super::{
        CapabilitySurfaceSync, build_core_tool_invokers, build_skill_catalog,
        build_stable_local_invokers,
    };
    use crate::{
        bootstrap::{
            capabilities::sync_external_tool_search_index,
            deps::core::{
                AstrError, CapabilityInvoker, CapabilityKind, CapabilitySpec,
                CapabilitySpecBuildError, Result,
            },
        },
        session_runtime_owner_bridge::ServerCapabilitySurfacePort,
        tool_capability_invoker::ToolCapabilityInvoker,
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
                continuation: None,
                error: None,
                metadata: None,
                duration_ms: 0,
                truncated: false,
            })
        }
    }

    fn invoker(name: &'static str, tags: &'static [&'static str]) -> Arc<dyn CapabilityInvoker> {
        Arc::new(
            ToolCapabilityInvoker::new(Arc::new(StaticTool { name, tags }))
                .expect("static tool should build"),
        ) as Arc<dyn CapabilityInvoker>
    }

    #[derive(Default)]
    struct TestCapabilitySurface {
        invokers: RwLock<Vec<Arc<dyn CapabilityInvoker>>>,
    }

    impl TestCapabilitySurface {
        fn capability_tool_names(&self) -> Vec<String> {
            self.invokers
                .read()
                .expect("test capability surface lock should not be poisoned")
                .iter()
                .map(|invoker| invoker.capability_spec().name.to_string())
                .collect()
        }
    }

    impl ServerCapabilitySurfacePort for TestCapabilitySurface {
        fn replace_capability_invokers(
            &self,
            invokers: Vec<Arc<dyn CapabilityInvoker>>,
        ) -> Result<()> {
            let mut seen = HashSet::new();
            for invoker in &invokers {
                let name = invoker.capability_spec().name.to_string();
                if !seen.insert(name.clone()) {
                    return Err(AstrError::Validation(format!(
                        "duplicate capability '{}'",
                        name
                    )));
                }
            }
            *self
                .invokers
                .write()
                .expect("test capability surface lock should not be poisoned") = invokers;
            Ok(())
        }
    }

    #[test]
    fn apply_external_invokers_keeps_previous_surface_on_failure() {
        let builtin_invoker = invoker("read_file", &["source:builtin"]);
        let core_tool_invokers = vec![builtin_invoker];
        let capability_surface = Arc::new(TestCapabilitySurface::default());
        let tool_search_index = Arc::new(ToolSearchIndex::new());
        let sync = CapabilitySurfaceSync::new(
            capability_surface,
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
        assert_eq!(sync.current_external_invokers().len(), 1);
        assert_eq!(
            sync.current_external_invokers()[0]
                .capability_spec()
                .name
                .as_str(),
            "mcp__demo__search"
        );
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
        let capability_surface = Arc::new(TestCapabilitySurface::default());
        let tool_search_index = Arc::new(ToolSearchIndex::new());
        let sync = CapabilitySurfaceSync::new(
            capability_surface.clone(),
            stable_local_invokers,
            Arc::clone(&tool_search_index),
        );

        sync.apply_external_invokers(vec![invoker("mcp__demo__search", &["source:mcp"])])
            .expect("replace should succeed");

        let names = capability_surface.capability_tool_names();
        assert!(names.iter().any(|name| name == "read_file"));
        assert!(names.iter().any(|name| name == "spawn"));
        assert!(names.iter().any(|name| name == "mcp__demo__search"));
    }

    #[test]
    fn build_core_tool_invokers_registers_task_write() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let tool_search_index = Arc::new(ToolSearchIndex::new());
        let skill_catalog =
            build_skill_catalog(temp.path(), Vec::new(), &ResourceCatalog::default());

        let invokers = build_core_tool_invokers(tool_search_index, skill_catalog)
            .expect("core tool invokers should build");
        let names = invokers
            .into_iter()
            .map(|invoker| invoker.capability_spec().name.to_string())
            .collect::<Vec<_>>();

        assert!(names.iter().any(|name| name == "taskWrite"));
    }
}
