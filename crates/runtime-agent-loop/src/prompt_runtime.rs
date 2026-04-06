//! # Prompt Runtime（提示运行时）
//!
//! ## 职责
//!
//! 桥接 `PromptComposer` 与 AgentLoop 的输入快照，负责在每个 step 中
//! 组装完整的系统提示词和规划结果（PromptPlan）。
//! 隔离了 turn_runner 与 prompt 贡献者/配置/skill 的复杂细节。
//!
//! ## 在 Turn 流程中的作用
//!
//! - **调用时机**：每个 step 开始构建请求时，`turn_runner` 调用 `build_plan()`
//! - **输入**：`AgentState`（会话状态）、`ConversationView`（模型可见消息）、`step_index`
//! - **输出**：`PromptBuildOutput`（包含组装好的 `PromptPlan` 和诊断信息）
//! - **缓存行为**：跨 step 复用 `PromptComposer` 的 KV cache 贡献者数据
//!
//! ## 依赖和协作
//!
//! - **使用** `astrcode_runtime_prompt::PromptComposer` 执行实际的提示词组装
//! - **使用** `CapabilityDescriptor` 列表供贡献者生成工具摘要
//! - **使用** `SkillCatalog` 实现两阶段 skill 暴露（索引 + 按需加载）
//! - **使用** `PromptDeclaration` 支持调用方注入自定义 prompt 块
//! - **被调用方**：`turn_runner` 在每个 step 中调用 `build_plan()`
//! - **输出给**：`RequestAssembler` 使用 `PromptPlan` 构建最终 LLM 请求
//!
//! ## 关键设计
//!
//! - `capability_descriptors()` 暴露与 prompt 一致的工具列表，供前端候选接口复用，避免漂移
//! - `skill_catalog()` 暴露统一的 skill 目录，供 skill tool 按需加载
//! - 持有 `tool_names` 列表，供 prompt composer 在环境变量块中注入工具名

use std::{collections::HashMap, sync::Arc};

use astrcode_core::{AgentState, AstrError, CapabilityDescriptor, Result};
use astrcode_runtime_agent_tool::AgentProfileCatalog;
use astrcode_runtime_prompt::{
    PromptAgentProfileSummary, PromptComposer, PromptContext, PromptDeclaration,
    PromptSkillSummary, composer::PromptBuildOutput,
};
use astrcode_runtime_skill_loader::SkillCatalog;

use crate::context_pipeline::ConversationView;

pub(crate) struct PromptRuntime {
    composer: PromptComposer,
    tool_names: Vec<String>,
    capability_descriptors: Vec<CapabilityDescriptor>,
    prompt_declarations: Vec<PromptDeclaration>,
    skill_catalog: Arc<SkillCatalog>,
    agent_profile_catalog: Option<Arc<dyn AgentProfileCatalog>>,
}

impl PromptRuntime {
    pub(crate) fn new(
        composer: PromptComposer,
        tool_names: Vec<String>,
        capability_descriptors: Vec<CapabilityDescriptor>,
        prompt_declarations: Vec<PromptDeclaration>,
        skill_catalog: Arc<SkillCatalog>,
        agent_profile_catalog: Option<Arc<dyn AgentProfileCatalog>>,
    ) -> Self {
        Self {
            composer,
            tool_names,
            capability_descriptors,
            prompt_declarations,
            skill_catalog,
            agent_profile_catalog,
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
            agent_profiles: self
                .agent_profile_catalog
                .as_ref()
                .map(|catalog| {
                    catalog
                        .list_subagent_profiles()
                        .into_iter()
                        .map(|profile| {
                            PromptAgentProfileSummary::new(profile.id, profile.description)
                        })
                        .collect()
                })
                .unwrap_or_default(),
            skills: self
                .skill_catalog
                .resolve_for_working_dir(&state.working_dir.to_string_lossy())
                .into_iter()
                .map(|skill| PromptSkillSummary::new(skill.id, skill.description))
                .collect(),
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
        },
    })
}
