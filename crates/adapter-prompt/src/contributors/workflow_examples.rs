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

const AGENT_COLLABORATION_TOOLS: &[&str] = &["spawn", "send", "observe", "close"];

#[async_trait]
impl PromptContributor for WorkflowExamplesContributor {
    fn contributor_id(&self) -> &'static str {
        "workflow-examples"
    }

    async fn contribute(&self, ctx: &PromptContext) -> PromptContribution {
        let mut blocks = vec![
            BlockSpec::message_text(
                "few-shot-user",
                BlockKind::FewShotExamples,
                "Few Shot User",
                "Before changing code, inspect the relevant files and gather context first. If \
                 you only know a filename pattern or glob, use `findFiles`. Use `grep` only when \
                 you have both a content pattern and a search path.",
                RenderTarget::PrependUser,
            )
            .with_condition(BlockCondition::FirstStepOnly)
            .with_priority(700),
            BlockSpec::message_text(
                "few-shot-assistant",
                BlockKind::FewShotExamples,
                "Few Shot Assistant",
                "I will inspect the relevant files and gather context before making changes. I \
                 will use `findFiles` to discover candidate paths, then use `grep` with both \
                 `pattern` and `path` when I need content search inside those files or \
                 directories.",
                RenderTarget::PrependAssistant,
            )
            .with_condition(BlockCondition::FirstStepOnly)
            .depends_on("few-shot-user")
            .with_priority(701),
        ];

        if has_agent_collaboration_tools(ctx) {
            blocks.push(
                BlockSpec::system_text(
                    "child-collaboration-guidance",
                    BlockKind::CollaborationGuide,
                    "Child Agent Collaboration Guide",
                    "When you receive a delivery from a child agent, use the four-tool \
                     collaboration model consistently. Treat the `agentId` returned by tool \
                     results as an exact opaque identifier. Copy it byte-for-byte in later \
                     `send`, `observe`, and `close` calls. Never renumber it, never zero-pad it, \
                     and never invent `agent-01` when the tool result says `agent-1`.\n\nChild \
                     agents are reusable. A child can finish one turn, return to `Idle`, and \
                     later receive more work through `send(agentId, message)`. Do not assume a \
                     completed turn means the child instance is gone.\n\nIf the delivery fully \
                     satisfies the original request, call `close(agentId)` to terminate the child \
                     subtree. If you need follow-up work or revisions, call `send(agentId, \
                     message)` with the next concrete instruction. If you are unsure whether the \
                     child is still running, idle, or already terminated, call `observe(agentId)` \
                     before deciding. Runtime mailbox delivery can be replayed after recovery. If \
                     you see the same `deliveryId` again, treat it as the same delivery rather \
                     than a new task.\n\nExample: if the tool result contains `agentId: agent-7`, \
                     every later collaboration call must use exactly `agent-7`. Default: close \
                     the child if the delivery satisfies the request; otherwise send a precise \
                     follow-up instruction or observe first.",
                )
                .with_priority(600),
            );
        }

        PromptContribution {
            blocks,
            ..PromptContribution::default()
        }
    }
}

fn has_agent_collaboration_tools(ctx: &PromptContext) -> bool {
    ctx.tool_names.iter().any(|tool_name| {
        AGENT_COLLABORATION_TOOLS
            .iter()
            .any(|candidate| tool_name == candidate)
    })
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
            capability_specs: Vec::new(),
            prompt_declarations: Vec::new(),
            agent_profiles: Vec::new(),
            skills: Vec::new(),
            step_index: 0,
            turn_index: 0,
            vars: Default::default(),
        };

        let output = composer.build(&ctx).await.expect("build should succeed");

        assert_eq!(output.plan.prepend_messages.len(), 2);
        assert!(
            output
                .plan
                .system_blocks
                .iter()
                .all(|block| block.id != "child-collaboration-guidance")
        );
        match &output.plan.prepend_messages[0] {
            LlmMessage::User { content, .. } => assert!(content.contains("findFiles")),
            other => panic!("expected prepended user message, got {other:?}"),
        }
        match &output.plan.prepend_messages[1] {
            LlmMessage::Assistant { content, .. } => assert!(content.contains("pattern")),
            other => panic!("expected prepended assistant message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn adds_collaboration_guidance_only_when_agent_tools_are_available() {
        let _guard = TestEnvGuard::new();
        let composer = PromptComposer::with_options(PromptComposerOptions {
            validation_level: ValidationLevel::Strict,
            ..PromptComposerOptions::default()
        });

        let ctx = PromptContext {
            working_dir: "/workspace/demo".to_string(),
            tool_names: vec![
                "shell".to_string(),
                "spawn".to_string(),
                "observe".to_string(),
            ],
            capability_specs: Vec::new(),
            prompt_declarations: Vec::new(),
            agent_profiles: Vec::new(),
            skills: Vec::new(),
            step_index: 0,
            turn_index: 0,
            vars: Default::default(),
        };

        let output = composer.build(&ctx).await.expect("build should succeed");

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
        assert!(
            collaboration_block
                .content
                .contains("`send`, `observe`, and `close`")
        );
        assert!(
            collaboration_block
                .content
                .contains("same `deliveryId` again")
        );
    }
}
