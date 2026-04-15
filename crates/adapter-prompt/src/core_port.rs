//! 桥接 `adapter-prompt` 的分层 prompt builder 与 `core::ports::PromptProvider`。
//!
//! `core::ports::PromptProvider` 是 kernel 消费的简化端口接口，
//! 本模块将其适配到 `LayeredPromptBuilder` 的完整 prompt 构建能力上。

use astrcode_core::{
    Result, SystemPromptBlock,
    ports::{PromptBuildCacheMetrics, PromptBuildOutput, PromptBuildRequest, PromptProvider},
};
use async_trait::async_trait;
use serde_json::Value;

use crate::{
    PromptAgentProfileSummary, PromptContext, PromptDeclaration, PromptSkillSummary,
    diagnostics::DiagnosticReason,
    layered_builder::{LayeredPromptBuilder, default_layered_prompt_builder},
};

/// 基于 `LayeredPromptBuilder` 的 `PromptProvider` 实现。
///
/// 将 `core::ports::PromptBuildRequest` 转为 `PromptContext`，
/// 调用分层 builder 后将 `PromptPlan` 渲染为 system prompt。
pub struct ComposerPromptProvider {
    builder: LayeredPromptBuilder,
}

impl ComposerPromptProvider {
    pub fn new(builder: LayeredPromptBuilder) -> Self {
        Self { builder }
    }

    /// 使用默认贡献者创建。
    pub fn with_defaults() -> Self {
        Self {
            builder: default_layered_prompt_builder(),
        }
    }
}

impl std::fmt::Debug for ComposerPromptProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComposerPromptProvider").finish()
    }
}

#[async_trait]
impl PromptProvider for ComposerPromptProvider {
    async fn build_prompt(&self, request: PromptBuildRequest) -> Result<PromptBuildOutput> {
        let vars = build_prompt_vars(&request);
        let ctx = PromptContext {
            working_dir: request.working_dir.to_string_lossy().to_string(),
            tool_names: request
                .capabilities
                .iter()
                .filter(|capability| capability.kind.is_tool())
                .map(|capability| capability.name.to_string())
                .collect(),
            capability_specs: request.capabilities,
            prompt_declarations: request
                .prompt_declarations
                .into_iter()
                .map(PromptDeclaration::from)
                .collect(),
            agent_profiles: request
                .agent_profiles
                .into_iter()
                .map(convert_agent_profile)
                .collect(),
            skills: request.skills.into_iter().map(convert_skill).collect(),
            step_index: request.step_index,
            turn_index: request.turn_index,
            vars,
        };

        let output = self
            .builder
            .build(&ctx)
            .await
            .map_err(|e| astrcode_core::AstrError::Internal(e.to_string()))?;

        let system_prompt = output.plan.render_system().unwrap_or_default();

        // 将 ordered system blocks 转为 core 的 SystemPromptBlock 格式
        let system_prompt_blocks: Vec<SystemPromptBlock> = output
            .plan
            .ordered_system_blocks()
            .into_iter()
            .map(|block| SystemPromptBlock {
                title: block.title.clone(),
                content: block.content.clone(),
                cache_boundary: false,
                layer: block.layer,
            })
            .collect();

        Ok(PromptBuildOutput {
            system_prompt,
            system_prompt_blocks,
            cache_metrics: summarize_prompt_cache_metrics(&output),
            metadata: serde_json::json!({
                "extra_tools_count": output.plan.extra_tools.len(),
                "diagnostics_count": output.diagnostics.items.len(),
                "profile": request.profile,
                "step_index": request.step_index,
                "turn_index": request.turn_index,
            }),
        })
    }
}

fn convert_agent_profile(
    summary: astrcode_core::PromptAgentProfileSummary,
) -> PromptAgentProfileSummary {
    PromptAgentProfileSummary::new(summary.id, summary.description)
}

fn convert_skill(summary: astrcode_core::PromptSkillSummary) -> PromptSkillSummary {
    PromptSkillSummary::new(summary.id, summary.description)
}

fn build_prompt_vars(request: &PromptBuildRequest) -> std::collections::HashMap<String, String> {
    let mut vars = std::collections::HashMap::new();
    if let Some(session_id) = &request.session_id {
        vars.insert("session.id".to_string(), session_id.to_string());
    }
    if let Some(turn_id) = &request.turn_id {
        vars.insert("turn.id".to_string(), turn_id.to_string());
    }
    vars.insert("profile.name".to_string(), request.profile.clone());
    insert_json_string(&mut vars, "profile.context", &request.profile_context);
    insert_json_string(&mut vars, "request.metadata", &request.metadata);
    if let Some(config_version) = request
        .metadata
        .get("configVersion")
        .and_then(Value::as_str)
    {
        vars.insert("config.version".to_string(), config_version.to_string());
    }
    if let Some(user_message) = request
        .metadata
        .get("latestUserMessage")
        .and_then(Value::as_str)
    {
        vars.insert("turn.user_message".to_string(), user_message.to_string());
    }
    if let Some(max_depth) = request
        .metadata
        .get("agentMaxSubrunDepth")
        .and_then(Value::as_u64)
    {
        vars.insert("agent.max_subrun_depth".to_string(), max_depth.to_string());
    }
    if let Some(max_spawn_per_turn) = request
        .metadata
        .get("agentMaxSpawnPerTurn")
        .and_then(Value::as_u64)
    {
        vars.insert(
            "agent.max_spawn_per_turn".to_string(),
            max_spawn_per_turn.to_string(),
        );
    }
    vars
}

fn summarize_prompt_cache_metrics(output: &crate::PromptBuildOutput) -> PromptBuildCacheMetrics {
    let mut metrics = PromptBuildCacheMetrics::default();
    for diagnostic in &output.diagnostics.items {
        match &diagnostic.reason {
            DiagnosticReason::CacheReuseHit { .. } => {
                metrics.reuse_hits = metrics.reuse_hits.saturating_add(1);
            },
            DiagnosticReason::CacheReuseMiss { .. } => {
                metrics.reuse_misses = metrics.reuse_misses.saturating_add(1);
            },
            _ => {},
        }
    }
    metrics
}

fn insert_json_string(
    vars: &mut std::collections::HashMap<String, String>,
    key: &str,
    value: &Value,
) {
    if value.is_null() {
        return;
    }
    let rendered = if let Some(text) = value.as_str() {
        text.to_string()
    } else {
        value.to_string()
    };
    vars.insert(key.to_string(), rendered);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use astrcode_core::ports::PromptBuildRequest;

    use super::build_prompt_vars;

    #[test]
    fn build_prompt_vars_exposes_agent_max_subrun_depth() {
        let request = PromptBuildRequest {
            session_id: None,
            turn_id: None,
            working_dir: PathBuf::from("/workspace/demo"),
            profile: "default".to_string(),
            step_index: 0,
            turn_index: 0,
            profile_context: serde_json::Value::Null,
            capabilities: Vec::new(),
            skills: Vec::new(),
            agent_profiles: Vec::new(),
            prompt_declarations: Vec::new(),
            metadata: serde_json::json!({
                "agentMaxSubrunDepth": 3u64,
            }),
        };

        let vars = build_prompt_vars(&request);

        assert_eq!(
            vars.get("agent.max_subrun_depth").map(String::as_str),
            Some("3")
        );
    }

    #[test]
    fn build_prompt_vars_exposes_agent_max_spawn_per_turn() {
        let request = PromptBuildRequest {
            session_id: None,
            turn_id: None,
            working_dir: PathBuf::from("/workspace/demo"),
            profile: "default".to_string(),
            step_index: 0,
            turn_index: 0,
            profile_context: serde_json::Value::Null,
            capabilities: Vec::new(),
            skills: Vec::new(),
            agent_profiles: Vec::new(),
            prompt_declarations: Vec::new(),
            metadata: serde_json::json!({
                "agentMaxSpawnPerTurn": 2u64,
            }),
        };

        let vars = build_prompt_vars(&request);

        assert_eq!(
            vars.get("agent.max_spawn_per_turn").map(String::as_str),
            Some("2")
        );
    }
}
