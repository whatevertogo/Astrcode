//! # Prompt 事实装配
//!
//! 将 prompt 组装依赖的 profile / skill / agent profile / prompt declaration
//! 收敛为稳定端口，避免 session-runtime 直接触碰 adapter 实现。

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use astrcode_adapter_agents::AgentProfileLoader;
use astrcode_adapter_mcp::manager::McpConnectionManager;
use astrcode_adapter_skills::SkillCatalog;
use astrcode_application::{ConfigService, resolve_current_model};
use astrcode_core::{
    PromptAgentProfileSummary, PromptDeclaration, PromptDeclarationKind,
    PromptDeclarationRenderTarget, PromptDeclarationSource, PromptFacts, PromptFactsProvider,
    PromptFactsRequest, PromptSkillSummary, Result, SystemPromptLayer, resolve_runtime_config,
};
use async_trait::async_trait;

pub(crate) fn build_prompt_facts_provider(
    config_service: Arc<ConfigService>,
    skill_catalog: Arc<SkillCatalog>,
    mcp_manager: Arc<McpConnectionManager>,
    agent_loader: AgentProfileLoader,
) -> Result<Arc<dyn PromptFactsProvider>> {
    Ok(Arc::new(RuntimePromptFactsProvider {
        config_service,
        skill_catalog,
        agent_loader,
        mcp_manager,
    }))
}

struct RuntimePromptFactsProvider {
    config_service: Arc<ConfigService>,
    skill_catalog: Arc<SkillCatalog>,
    agent_loader: AgentProfileLoader,
    mcp_manager: Arc<McpConnectionManager>,
}

impl std::fmt::Debug for RuntimePromptFactsProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimePromptFactsProvider")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl PromptFactsProvider for RuntimePromptFactsProvider {
    async fn resolve_prompt_facts(&self, request: &PromptFactsRequest) -> Result<PromptFacts> {
        let working_dir = request.working_dir.clone();
        let config = self
            .config_service
            .load_overlayed_config(Some(working_dir.as_path()))
            .map_err(|error| astrcode_core::AstrError::Internal(error.to_string()))?;
        let runtime = resolve_runtime_config(&config.runtime);
        let selection = resolve_current_model(&config)
            .map_err(|error| astrcode_core::AstrError::Internal(error.to_string()))?;
        let skill_summaries = self
            .skill_catalog
            .resolve_for_working_dir(&working_dir.to_string_lossy())
            .into_iter()
            .map(|skill| PromptSkillSummary {
                id: skill.id,
                description: skill.description,
            })
            .collect();
        let agent_profiles = self
            .agent_loader
            .load_for_working_dir(Some(working_dir.as_path()))
            .map_err(|error| astrcode_core::AstrError::Internal(error.to_string()))?
            .list_subagent_profiles()
            .into_iter()
            .map(|profile| PromptAgentProfileSummary {
                id: profile.id.clone(),
                description: profile.description.clone(),
            })
            .collect();
        let prompt_declarations = self
            .mcp_manager
            .current_surface()
            .await
            .prompt_declarations
            .into_iter()
            .map(convert_prompt_declaration)
            .collect();

        Ok(PromptFacts {
            profile: selection.profile_name,
            profile_context: build_profile_context(
                working_dir.as_path(),
                request.session_id.as_ref().map(ToString::to_string),
                request.turn_id.as_ref().map(ToString::to_string),
            ),
            metadata: serde_json::json!({
                "configVersion": config.version,
                "activeProfile": config.active_profile,
                "activeModel": config.active_model,
                "agentMaxSubrunDepth": runtime.agent.max_subrun_depth,
            }),
            skills: skill_summaries,
            agent_profiles,
            prompt_declarations,
        })
    }
}

fn build_profile_context(
    working_dir: &Path,
    session_id: Option<String>,
    turn_id: Option<String>,
) -> serde_json::Value {
    let working_dir = normalize_context_path(working_dir);
    let mut context = serde_json::Map::new();
    context.insert(
        "workingDir".to_string(),
        serde_json::Value::String(working_dir.clone()),
    );
    context.insert(
        "repoRoot".to_string(),
        serde_json::Value::String(working_dir),
    );
    context.insert(
        "approvalMode".to_string(),
        serde_json::Value::String("inherit".to_string()),
    );
    if let Some(session_id) = session_id {
        context.insert(
            "sessionId".to_string(),
            serde_json::Value::String(session_id),
        );
    }
    if let Some(turn_id) = turn_id {
        context.insert("turnId".to_string(), serde_json::Value::String(turn_id));
    }
    serde_json::Value::Object(context)
}

fn normalize_context_path(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| PathBuf::from(path))
        .to_string_lossy()
        .into_owned()
}

fn convert_prompt_declaration(
    declaration: astrcode_adapter_prompt::PromptDeclaration,
) -> PromptDeclaration {
    PromptDeclaration {
        block_id: declaration.block_id,
        title: declaration.title,
        content: declaration.content,
        render_target: match declaration.render_target {
            astrcode_adapter_prompt::PromptDeclarationRenderTarget::System => {
                PromptDeclarationRenderTarget::System
            },
            astrcode_adapter_prompt::PromptDeclarationRenderTarget::PrependUser => {
                PromptDeclarationRenderTarget::PrependUser
            },
            astrcode_adapter_prompt::PromptDeclarationRenderTarget::PrependAssistant => {
                PromptDeclarationRenderTarget::PrependAssistant
            },
            astrcode_adapter_prompt::PromptDeclarationRenderTarget::AppendUser => {
                PromptDeclarationRenderTarget::AppendUser
            },
            astrcode_adapter_prompt::PromptDeclarationRenderTarget::AppendAssistant => {
                PromptDeclarationRenderTarget::AppendAssistant
            },
        },
        layer: match declaration.layer {
            astrcode_adapter_prompt::PromptLayer::Stable => SystemPromptLayer::Stable,
            astrcode_adapter_prompt::PromptLayer::SemiStable => SystemPromptLayer::SemiStable,
            astrcode_adapter_prompt::PromptLayer::Inherited => SystemPromptLayer::Inherited,
            astrcode_adapter_prompt::PromptLayer::Dynamic => SystemPromptLayer::Dynamic,
            astrcode_adapter_prompt::PromptLayer::Unspecified => SystemPromptLayer::Unspecified,
        },
        kind: match declaration.kind {
            astrcode_adapter_prompt::PromptDeclarationKind::ToolGuide => {
                PromptDeclarationKind::ToolGuide
            },
            astrcode_adapter_prompt::PromptDeclarationKind::ExtensionInstruction => {
                PromptDeclarationKind::ExtensionInstruction
            },
        },
        priority_hint: declaration.priority_hint,
        always_include: declaration.always_include,
        source: match declaration.source {
            astrcode_adapter_prompt::PromptDeclarationSource::Builtin => {
                PromptDeclarationSource::Builtin
            },
            astrcode_adapter_prompt::PromptDeclarationSource::Plugin => {
                PromptDeclarationSource::Plugin
            },
            astrcode_adapter_prompt::PromptDeclarationSource::Mcp => PromptDeclarationSource::Mcp,
        },
        capability_name: declaration.capability_name,
        origin: declaration.origin,
    }
}
