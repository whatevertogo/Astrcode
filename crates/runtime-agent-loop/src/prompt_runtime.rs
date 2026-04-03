//! 提示运行时适配器
//!
//! 此包装器将提示特定的输入和转换保持在 `runtime-prompt` 附近
//! 而不让 `turn_runner` 直接接触贡献者/配置/技能的详细信息

use std::collections::HashMap;
use std::sync::Arc;

use astrcode_core::{AgentState, AstrError, CapabilityDescriptor, Result};
use astrcode_runtime_prompt::{
    composer::PromptBuildOutput, PromptComposer, PromptContext, PromptDeclaration,
};
use astrcode_runtime_skill_loader::SkillCatalog;

use crate::context_pipeline::ConversationView;

pub(crate) struct PromptRuntime {
    composer: PromptComposer,
    tool_names: Vec<String>,
    capability_descriptors: Vec<CapabilityDescriptor>,
    prompt_declarations: Vec<PromptDeclaration>,
    skill_catalog: Arc<SkillCatalog>,
}

impl PromptRuntime {
    pub(crate) fn new(
        composer: PromptComposer,
        tool_names: Vec<String>,
        capability_descriptors: Vec<CapabilityDescriptor>,
        prompt_declarations: Vec<PromptDeclaration>,
        skill_catalog: Arc<SkillCatalog>,
    ) -> Self {
        Self {
            composer,
            tool_names,
            capability_descriptors,
            prompt_declarations,
            skill_catalog,
        }
    }

    pub(crate) fn capability_descriptors(&self) -> &[CapabilityDescriptor] {
        &self.capability_descriptors
    }

    pub(crate) fn skill_catalog(&self) -> Arc<SkillCatalog> {
        Arc::clone(&self.skill_catalog)
    }

    #[cfg(test)]
    pub(crate) fn with_composer(mut self, composer: PromptComposer) -> Self {
        self.composer = composer;
        self
    }

    /// Build a prompt plan from the loop-provided conversation snapshot.
    pub(crate) async fn build_plan(
        &self,
        state: &AgentState,
        conversation: &ConversationView,
        step_index: usize,
    ) -> Result<PromptBuildOutput> {
        let mut vars = HashMap::new();
        if let Some(latest_user_message) = latest_user_message(&conversation.messages) {
            vars.insert(
                "turn.user_message".to_string(),
                latest_user_message.to_string(),
            );
        }
        let ctx = PromptContext {
            working_dir: state.working_dir.to_string_lossy().into_owned(),
            tool_names: self.tool_names.clone(),
            capability_descriptors: self.capability_descriptors.clone(),
            prompt_declarations: self.prompt_declarations.clone(),
            skills: self
                .skill_catalog
                .resolve_for_working_dir(&state.working_dir.to_string_lossy()),
            step_index,
            turn_index: state.turn_count,
            vars,
        };
        self.composer
            .build(&ctx)
            .await
            .map_err(|error| AstrError::Internal(error.to_string()))
    }
}

fn latest_user_message(messages: &[astrcode_core::LlmMessage]) -> Option<&str> {
    messages.iter().rev().find_map(|message| match message {
        astrcode_core::LlmMessage::User { content, .. } => Some(content.as_str()),
        astrcode_core::LlmMessage::Assistant { .. } | astrcode_core::LlmMessage::Tool { .. } => {
            None
        }
    })
}
