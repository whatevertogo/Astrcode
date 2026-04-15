//! 工作流示例贡献者。
//!
//! 提供 few-shot 示例对话，教导模型"先收集上下文再修改代码"的行为模式。
//! 仅在第一步（step_index == 0）时生效，以 prepend 方式插入到对话消息中。
//!
//! 同时提供子 Agent 协作决策指导：当父 Agent 收到子 Agent 交付结果后，
//! 指导模型如何决定关闭或保留子 Agent。

use astrcode_core::config::DEFAULT_MAX_SUBRUN_DEPTH;
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

        if should_add_tool_search_example(ctx) {
            blocks.push(
                BlockSpec::message_text(
                    "tool-search-few-shot-user",
                    BlockKind::FewShotExamples,
                    "Tool Search Few Shot User",
                    "A visible external `mcp__...` tool looks relevant, but its parameters are \
                     unclear. Do not guess argument names or call it with an empty object. Use \
                     `tool_search` first to inspect the external tool schema.",
                    RenderTarget::PrependUser,
                )
                .with_condition(BlockCondition::FirstStepOnly)
                .with_priority(702),
            );
            blocks.push(
                BlockSpec::message_text(
                    "tool-search-few-shot-assistant",
                    BlockKind::FewShotExamples,
                    "Tool Search Few Shot Assistant",
                    "I will not guess parameters for the external tool. I will call `tool_search` \
                     first with part of the tool name or task purpose, for example `{ \"query\": \
                     \"webReader\" }` or `{ \"query\": \"github repo structure\" }`, read the \
                     returned `inputSchema`, and only then call the matching `mcp__...` tool with \
                     the documented arguments.",
                    RenderTarget::PrependAssistant,
                )
                .with_condition(BlockCondition::FirstStepOnly)
                .depends_on("tool-search-few-shot-user")
                .with_priority(703),
            );
        }

        if has_agent_collaboration_tools(ctx) {
            let max_depth = collaboration_depth_limit(ctx).unwrap_or(DEFAULT_MAX_SUBRUN_DEPTH);
            let max_spawn_per_turn = collaboration_spawn_limit(ctx).unwrap_or(3);
            blocks.push(
                BlockSpec::system_text(
                    "child-collaboration-guidance",
                    BlockKind::CollaborationGuide,
                    "Child Agent Collaboration Guide",
                    format!(
                        "Use the child-agent tools as one decision protocol.\n\nKeep `agentId` \
                         exact. Copy it byte-for-byte in later `send`, `observe`, and `close` \
                         calls. Never renumber it, never zero-pad it, and never invent `agent-01` \
                         when the tool result says `agent-1`.\n\nDefault protocol:\n1. `spawn` \
                         only for a new isolated responsibility with real parallel or \
                         context-isolation value.\n2. `send` when the same child should take one \
                         concrete next step on the same responsibility branch.\n3. `observe` only \
                         when the next decision depends on current child state.\n4. `close` when \
                         the branch is done or no longer useful.\n\nDelegation modes:\n- Fresh \
                         child: use `spawn` for a new responsibility branch. Give the child a \
                         full briefing: task scope, boundaries, expected deliverable, and any \
                         important focus or exclusion. Do not treat a short nudge like 'take a \
                         look' as a sufficient fresh-child brief.\n- Resumed child: use `send` \
                         when the same child should continue the same responsibility branch. Send \
                         one concrete delta instruction or clarification, not a full re-briefing \
                         of the original task.\n- Restricted child: when you narrow a child with \
                         `capabilityGrant`, assign only work that fits that reduced capability \
                         surface. If the next step needs tools the restricted child does not \
                         have, choose a different child or do the work locally instead of forcing \
                         a mismatch.\n\n`Idle` is normal and reusable. Do not respawn just \
                         because a child finished one turn. Reuse an idle child with \
                         `send(agentId, message)` when the responsibility stays the same. If you \
                         are unsure whether the child is still running, idle, or terminated, call \
                         `observe(agentId)` once and act on the result.\n\nSpawn sparingly. The \
                         runtime enforces a maximum child depth of {max_depth} and at most \
                         {max_spawn_per_turn} new children per turn. Start with one child unless \
                         there are clearly separate workstreams. Do not blanket-spawn agents just \
                         to explore a repo broadly.\n\nAvoid waste:\n- Do not loop on `observe` \
                         with no decision attached.\n- If a child is still running and you are \
                         simply waiting, use your current shell tool to sleep briefly instead of \
                         spending another tool call on `observe`.\n- Do not stack speculative \
                         `send` calls.\n- Do not spawn a new child when an existing idle child \
                         already owns the responsibility.\n\nIf a delivery satisfies the request, \
                         `close` the branch. If the same child should continue, `send` one \
                         precise follow-up. If you see the same `deliveryId` again after \
                         recovery, treat it as the same delivery, not a new task.\n\nWhen you are \
                         the child on a delegated task, use upstream `send(kind + payload)` to \
                         deliver a formal message to your direct parent. Report `progress`, \
                         `completed`, `failed`, or `close_request` explicitly. Do not wait for \
                         the parent to infer state from raw intermediate steps, and do not end \
                         with an open loop like '继续观察中' unless you are also sending a \
                         non-terminal `progress` delivery that keeps the branch alive.\n\nWhen \
                         you are the parent and receive a child delivery, treat it as a decision \
                         point. Do not leave it hanging and do not immediately re-observe the \
                         same child unless the state is unclear. Decide immediately whether the \
                         result is complete enough to `close` the branch, or whether the same \
                         child should continue with one concrete `send` follow-up that names the \
                         exact next step."
                    ),
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

fn should_add_tool_search_example(ctx: &PromptContext) -> bool {
    has_tool_search(ctx) && has_external_tools(ctx)
}

fn collaboration_depth_limit(ctx: &PromptContext) -> Option<usize> {
    ctx.vars
        .get("agent.max_subrun_depth")
        .and_then(|value| value.parse::<usize>().ok())
}

fn collaboration_spawn_limit(ctx: &PromptContext) -> Option<usize> {
    ctx.vars
        .get("agent.max_spawn_per_turn")
        .and_then(|value| value.parse::<usize>().ok())
}

fn has_tool_search(ctx: &PromptContext) -> bool {
    ctx.tool_names
        .iter()
        .any(|tool_name| tool_name == "tool_search")
}

fn has_external_tools(ctx: &PromptContext) -> bool {
    ctx.capability_specs.iter().any(|spec| {
        spec.kind.is_tool()
            && spec
                .tags
                .iter()
                .any(|tag| tag == "source:mcp" || tag == "source:plugin")
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
    async fn adds_tool_search_examples_when_external_tools_are_available() {
        let _guard = TestEnvGuard::new();
        let composer = PromptComposer::with_options(PromptComposerOptions {
            validation_level: ValidationLevel::Strict,
            ..PromptComposerOptions::default()
        });

        let ctx = PromptContext {
            working_dir: "/workspace/demo".to_string(),
            tool_names: vec![
                "tool_search".to_string(),
                "mcp__web-reader__webReader".to_string(),
            ],
            capability_specs: vec![
                astrcode_core::CapabilitySpec::builder(
                    "mcp__web-reader__webReader",
                    astrcode_core::CapabilityKind::Tool,
                )
                .description("Fetch and Convert URL to Large Model Friendly Input.")
                .schema(
                    serde_json::json!({"type": "object"}),
                    serde_json::json!({"type": "string"}),
                )
                .tags(["source:mcp"])
                .build()
                .expect("spec should build"),
            ],
            prompt_declarations: Vec::new(),
            agent_profiles: Vec::new(),
            skills: Vec::new(),
            step_index: 0,
            turn_index: 0,
            vars: Default::default(),
        };

        let output = composer.build(&ctx).await.expect("build should succeed");

        assert_eq!(output.plan.prepend_messages.len(), 4);
        match &output.plan.prepend_messages[2] {
            LlmMessage::User { content, .. } => {
                assert!(content.contains("Do not guess argument names"));
                assert!(content.contains("`tool_search`"));
            },
            other => panic!("expected prepended user message, got {other:?}"),
        }
        match &output.plan.prepend_messages[3] {
            LlmMessage::Assistant { content, .. } => {
                assert!(content.contains("{ \"query\": \"webReader\" }"));
                assert!(content.contains("`inputSchema`"));
            },
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
        assert!(collaboration_block.content.contains("Keep `agentId` exact"));
        assert!(collaboration_block.content.contains(&format!(
            "maximum child depth of {DEFAULT_MAX_SUBRUN_DEPTH}"
        )));
        assert!(collaboration_block.content.contains("Default protocol:"));
        assert!(collaboration_block.content.contains("Delegation modes:"));
        assert!(collaboration_block.content.contains("Fresh child:"));
        assert!(collaboration_block.content.contains("Resumed child:"));
        assert!(collaboration_block.content.contains("Restricted child:"));
        assert!(
            collaboration_block
                .content
                .contains("same `deliveryId` again")
        );
        assert!(
            collaboration_block
                .content
                .contains("`Idle` is normal and reusable")
        );
        assert!(
            collaboration_block
                .content
                .contains("Do not loop on `observe`")
        );
        assert!(
            collaboration_block
                .content
                .contains("use upstream `send(kind + payload)`")
        );
        assert!(collaboration_block.content.contains("`close_request`"));
        assert!(collaboration_block.content.contains("exact next step"));
    }

    #[tokio::test]
    async fn collaboration_guidance_uses_configured_depth_limit() {
        let _guard = TestEnvGuard::new();
        let composer = PromptComposer::with_options(PromptComposerOptions {
            validation_level: ValidationLevel::Strict,
            ..PromptComposerOptions::default()
        });

        let mut vars = std::collections::HashMap::new();
        vars.insert("agent.max_subrun_depth".to_string(), "5".to_string());
        let ctx = PromptContext {
            working_dir: "/workspace/demo".to_string(),
            tool_names: vec![
                "spawn".to_string(),
                "send".to_string(),
                "observe".to_string(),
                "close".to_string(),
            ],
            capability_specs: Vec::new(),
            prompt_declarations: Vec::new(),
            agent_profiles: Vec::new(),
            skills: Vec::new(),
            step_index: 0,
            turn_index: 0,
            vars,
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
                .contains("maximum child depth of 5")
        );
    }
}
