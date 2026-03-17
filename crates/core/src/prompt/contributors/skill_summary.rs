use async_trait::async_trait;

use crate::prompt::{
    BlockCondition, BlockKind, BlockSpec, PromptContext, PromptContribution, PromptContributor,
    RenderTarget,
};

pub struct SkillSummaryContributor;

#[async_trait]
impl PromptContributor for SkillSummaryContributor {
    fn contributor_id(&self) -> &'static str {
        "skill-summary"
    }

    async fn contribute(&self, _ctx: &PromptContext) -> PromptContribution {
        PromptContribution {
            blocks: vec![
                BlockSpec::system_template(
                    "skill-summary",
                    BlockKind::Skill,
                    "Skill Summary",
                    "Available tools: {{tools.names}}",
                )
                .with_tag("skills")
                .with_category("capabilities"),
                BlockSpec::message_text(
                    "few-shot-user",
                    BlockKind::FewShotExamples,
                    "Few Shot User",
                    "Before changing code, inspect the relevant files and gather context first.",
                    RenderTarget::PrependUser,
                )
                .with_condition(BlockCondition::FirstStepOnly)
                .with_priority(700),
                BlockSpec::message_text(
                    "few-shot-assistant",
                    BlockKind::FewShotExamples,
                    "Few Shot Assistant",
                    "I will inspect the relevant files and gather context before making changes.",
                    RenderTarget::PrependAssistant,
                )
                .with_condition(BlockCondition::FirstStepOnly)
                .depends_on("few-shot-user")
                .with_priority(701),
            ],
            ..PromptContribution::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{OrchestrationConfig, ValidationLevel};
    use crate::prompt::PromptComposer;

    #[tokio::test]
    async fn adds_skill_summary_and_first_step_examples() {
        let composer = PromptComposer::new(OrchestrationConfig {
            validation_strictness: ValidationLevel::Strict,
            ..OrchestrationConfig::default()
        })
        .add(std::sync::Arc::new(SkillSummaryContributor));

        let ctx = PromptContext {
            working_dir: "/workspace/demo".to_string(),
            tool_names: vec!["shell".to_string()],
            step_index: 0,
            turn_index: 0,
            vars: Default::default(),
        };

        let output = composer.build(&ctx).await.expect("build should succeed");

        assert!(output
            .plan
            .system_blocks
            .iter()
            .any(|block| block.kind == BlockKind::Skill));
        assert_eq!(output.plan.prepend_messages.len(), 2);
    }
}
