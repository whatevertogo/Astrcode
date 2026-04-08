//! 工作流示例贡献者。
//!
//! 提供 few-shot 示例对话，教导模型"先收集上下文再修改代码"的行为模式。
//! 仅在第一步（step_index == 0）时生效，以 prepend 方式插入到对话消息中。
//!
//! 同时提供子 Agent 协作决策指导：当父 Agent 收到子 Agent 交付结果后，
//! 指导模型如何决定关闭或保留子 Agent。

use async_trait::async_trait;

use crate::{
    BlockCondition, BlockKind, BlockSpec, PromptContext, PromptContribution, PromptContributor,
    RenderTarget,
};

pub struct WorkflowExamplesContributor;

#[async_trait]
impl PromptContributor for WorkflowExamplesContributor {
    fn contributor_id(&self) -> &'static str {
        "workflow-examples"
    }

    async fn contribute(&self, _ctx: &PromptContext) -> PromptContribution {
        PromptContribution {
            blocks: vec![
                BlockSpec::message_text(
                    "few-shot-user",
                    BlockKind::FewShotExamples,
                    "Few Shot User",
                    "Before changing code, inspect the relevant files and gather context first. \
                     If you only know a filename pattern or glob, use `findFiles`. Use `grep` \
                     only when you have both a content pattern and a search path.",
                    RenderTarget::PrependUser,
                )
                .with_condition(BlockCondition::FirstStepOnly)
                .with_priority(700),
                BlockSpec::message_text(
                    "few-shot-assistant",
                    BlockKind::FewShotExamples,
                    "Few Shot Assistant",
                    "I will inspect the relevant files and gather context before making changes. \
                     I will use `findFiles` to discover candidate paths, then use `grep` with \
                     both `pattern` and `path` when I need content search inside those files or \
                     directories.",
                    RenderTarget::PrependAssistant,
                )
                .with_condition(BlockCondition::FirstStepOnly)
                .depends_on("few-shot-user")
                .with_priority(701),
                // 子 Agent 协作决策指导
                BlockSpec::system_text(
                    "child-collaboration-guidance",
                    BlockKind::CollaborationGuide,
                    "Child Agent Collaboration Guide",
                    "When you receive a delivery from a child agent, decide whether to close or \
                     keep the child: - Treat the `agentId` returned by tool results as an exact \
                     opaque identifier. Copy it byte-for-byte in later `waitAgent`, `sendAgent`, \
                     `closeAgent`, and `resumeAgent` calls. Never renumber it, never zero-pad it, \
                     and never invent `agent-01` when the tool result says `agent-1`. - Use \
                     `closeAgent` if the child's task is fully complete and no further work is \
                     needed. Set `cascade: true` to close the entire subtree, or `cascade: false` \
                     to close only the target agent. - Use `sendAgent` if you need the child to \
                     continue with follow-up work or revisions based on the delivery. - Use \
                     `waitAgent` if you want to wait for the child's next delivery before \
                     proceeding. Example: if the tool result contains `Child agent reference` \
                     with `agentId: agent-7`, the next collaboration call must use exactly \
                     `agent-7`. Default: close the child if the delivery satisfies the original \
                     request; keep it running if you need additional iterations.",
                )
                .with_priority(600),
            ],
            ..PromptContribution::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{LlmMessage, test_support::TestEnvGuard};

    use super::*;
    use crate::{PromptComposer, PromptComposerOptions, ValidationLevel};

    #[tokio::test]
    async fn adds_first_step_examples() {
        let _guard = TestEnvGuard::new();
        let composer = PromptComposer::with_options(PromptComposerOptions {
            validation_level: ValidationLevel::Strict,
            ..PromptComposerOptions::default()
        });

        let ctx = PromptContext {
            working_dir: "/workspace/demo".to_string(),
            tool_names: vec!["shell".to_string()],
            capability_descriptors: Vec::new(),
            prompt_declarations: Vec::new(),
            agent_profiles: Vec::new(),
            skills: Vec::new(),
            step_index: 0,
            turn_index: 0,
            vars: Default::default(),
        };

        let output = composer.build(&ctx).await.expect("build should succeed");

        assert_eq!(output.plan.prepend_messages.len(), 2);
        let collaboration_block = output
            .plan
            .system_blocks
            .iter()
            .find(|block| block.id == "child-collaboration-guidance")
            .expect("collaboration guidance block should exist");
        assert!(
            collaboration_block
                .content
                .contains("Copy it byte-for-byte")
        );
        assert!(collaboration_block.content.contains("agent-01"));
        match &output.plan.prepend_messages[0] {
            LlmMessage::User { content, .. } => assert!(content.contains("findFiles")),
            other => panic!("expected prepended user message, got {other:?}"),
        }
        match &output.plan.prepend_messages[1] {
            LlmMessage::Assistant { content, .. } => assert!(content.contains("pattern")),
            other => panic!("expected prepended assistant message, got {other:?}"),
        }
    }
}
