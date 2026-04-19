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
use astrcode_application::config::{ConfigService, resolve_current_model};
use astrcode_core::SkillCatalog;
use async_trait::async_trait;

use super::deps::core::{
    AstrError, PromptDeclaration, PromptDeclarationKind, PromptDeclarationRenderTarget,
    PromptDeclarationSource, PromptEntrySummary, PromptFacts, PromptFactsProvider,
    PromptFactsRequest, Result, SystemPromptLayer, resolve_runtime_config,
};

pub(crate) fn build_prompt_facts_provider(
    config_service: Arc<ConfigService>,
    skill_catalog: Arc<dyn SkillCatalog>,
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
    skill_catalog: Arc<dyn SkillCatalog>,
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
            .map_err(|error| AstrError::Internal(error.to_string()))?;
        let runtime = resolve_runtime_config(&config.runtime);
        let governance = request.governance.clone().unwrap_or_default();
        let selection = resolve_current_model(&config)
            .map_err(|error| AstrError::Internal(error.to_string()))?;
        let skill_summaries = self
            .skill_catalog
            .resolve_for_working_dir(&working_dir.to_string_lossy())
            .into_iter()
            .map(|skill| PromptEntrySummary::new(skill.id, skill.description))
            .collect();
        let agent_profiles = self
            .agent_loader
            .load_for_working_dir(Some(working_dir.as_path()))
            .map_err(|error| AstrError::Internal(error.to_string()))?
            .list_subagent_profiles()
            .into_iter()
            .map(|profile| PromptEntrySummary::new(profile.id.clone(), profile.description.clone()))
            .collect();
        let prompt_declarations = self
            .mcp_manager
            .current_surface()
            .await
            .prompt_declarations
            .into_iter()
            .filter(|declaration| {
                prompt_declaration_is_visible(
                    governance.allowed_capability_names.as_slice(),
                    declaration,
                )
            })
            .map(convert_prompt_declaration)
            .collect();

        Ok(PromptFacts {
            profile: selection.profile_name,
            profile_context: build_profile_context(
                working_dir.as_path(),
                request.session_id.as_ref().map(ToString::to_string),
                request.turn_id.as_ref().map(ToString::to_string),
                governance.approval_mode.as_str(),
            ),
            metadata: serde_json::json!({
                "configVersion": config.version,
                "activeProfile": config.active_profile,
                "activeModel": config.active_model,
                "modeId": governance.mode_id,
                "agentMaxSubrunDepth": governance.max_subrun_depth.unwrap_or(runtime.agent.max_subrun_depth),
                "agentMaxSpawnPerTurn": governance.max_spawn_per_turn.unwrap_or(runtime.agent.max_spawn_per_turn),
                "governancePolicyRevision": governance.policy_revision,
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
    approval_mode: &str,
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
        serde_json::Value::String(if approval_mode.trim().is_empty() {
            "inherit".to_string()
        } else {
            approval_mode.to_string()
        }),
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

fn prompt_declaration_is_visible(
    allowed_capability_names: &[String],
    declaration: &astrcode_adapter_prompt::PromptDeclaration,
) -> bool {
    declaration
        .capability_name
        .as_ref()
        .is_none_or(|capability_name| {
            allowed_capability_names
                .iter()
                .any(|allowed| allowed == capability_name)
        })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use astrcode_adapter_prompt::{
        PromptDeclaration as AdapterPromptDeclaration, PromptDeclarationKind,
        PromptDeclarationRenderTarget, PromptDeclarationSource, PromptLayer,
    };

    use super::prompt_declaration_is_visible;
    use crate::bootstrap::deps::core::PromptFactsRequest;

    fn declaration(capability_name: Option<&str>) -> AdapterPromptDeclaration {
        AdapterPromptDeclaration {
            block_id: "tool-guide".to_string(),
            title: "Tool Guide".to_string(),
            content: "only visible for allowed capabilities".to_string(),
            render_target: PromptDeclarationRenderTarget::System,
            layer: PromptLayer::Dynamic,
            kind: PromptDeclarationKind::ToolGuide,
            priority_hint: None,
            always_include: false,
            source: PromptDeclarationSource::Mcp,
            capability_name: capability_name.map(ToString::to_string),
            origin: Some("test".to_string()),
        }
    }

    fn request(allowed_capability_names: &[&str]) -> PromptFactsRequest {
        PromptFactsRequest {
            session_id: None,
            turn_id: None,
            working_dir: PathBuf::from("."),
            allowed_capability_names: allowed_capability_names
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
            governance: Some(astrcode_core::PromptGovernanceContext {
                allowed_capability_names: allowed_capability_names
                    .iter()
                    .map(|name| (*name).to_string())
                    .collect(),
                mode_id: Some(astrcode_core::ModeId::code()),
                approval_mode: "inherit".to_string(),
                policy_revision: "governance-surface-v1".to_string(),
                max_subrun_depth: Some(3),
                max_spawn_per_turn: Some(2),
            }),
        }
    }

    #[test]
    fn prompt_declaration_visibility_keeps_capabilityless_declarations() {
        assert!(prompt_declaration_is_visible(
            &request(&[]).governance.unwrap().allowed_capability_names,
            &declaration(None)
        ));
    }

    #[test]
    fn prompt_declaration_visibility_filters_out_ungranted_capabilities() {
        assert!(!prompt_declaration_is_visible(
            &request(&["readFile"])
                .governance
                .unwrap()
                .allowed_capability_names,
            &declaration(Some("spawn"))
        ));
    }

    #[test]
    fn prompt_declaration_visibility_keeps_granted_capabilities() {
        assert!(prompt_declaration_is_visible(
            &request(&["spawn"])
                .governance
                .unwrap()
                .allowed_capability_names,
            &declaration(Some("spawn"))
        ));
    }
}
