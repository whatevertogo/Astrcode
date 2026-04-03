//！请求程序集边界。
//！
//！及时规划决定指令支架，上下文规划决定模型可见
//！对话，该模块执行两者之间的最终请求编码步骤

use astrcode_core::{CapabilityDescriptor, LlmMessage, ModelRequest, Result, ToolDefinition};
use astrcode_runtime_prompt::{append_unique_tools, PromptPlan};

use crate::context_pipeline::{ContextBundle, ConversationView};
use crate::context_window::{
    apply_microcompact, build_prompt_snapshot, effective_context_window, MicrocompactResult,
    PromptTokenSnapshot, TokenUsageTracker,
};

pub(crate) struct RequestAssembler;

/// Prepared request plus request-shape diagnostics needed by the loop.
pub(crate) struct PreparedRequest {
    pub request: ModelRequest,
    pub prompt_snapshot: PromptTokenSnapshot,
    pub truncated_tool_results: usize,
}

/// Microcompact configuration carried through request preparation.
pub(crate) struct MicrocompactConfig<'a> {
    pub capability_descriptors: &'a [CapabilityDescriptor],
    pub tool_result_max_bytes: usize,
    pub keep_recent_turns: usize,
    pub model_context_window: usize,
    pub compact_threshold_percent: u8,
}

/// Configuration for rebuilding a model request after compaction.
///
/// Bundles the parameters that `rebuild_request_after_compaction` needs
/// to reconstruct the model-visible messages while preserving request-level
/// metadata (system prompt, tools, etc.).
pub(crate) struct CompactionRebuildConfig<'a> {
    pub capability_descriptors: &'a [CapabilityDescriptor],
    pub tool_result_max_bytes: usize,
    pub keep_recent_turns: usize,
    pub model_context_window: usize,
}

/// Inputs needed to build and prepare a step request in one place.
pub(crate) struct StepRequestConfig<'a> {
    pub prompt: &'a PromptPlan,
    pub context: ContextBundle,
    pub tools: Vec<ToolDefinition>,
    pub microcompact: MicrocompactConfig<'a>,
}

impl RequestAssembler {
    /// 组装最终请求结构并立即运行请求级预处理。
    ///
    /// 将这两个步骤保持在一起可以防止 `turn_runner` 拥有消息组合、工具联合、微压缩和提示快照生成的确切顺序
    /// 这些都是紧密耦合的细节，未来可能会频繁调整
    /// 将它们封装在 `RequestAssembler` 内部允许我们在不影响 `turn_runner` 的情况下迭代这些细节
    pub(crate) fn build_step_request(
        &self,
        config: StepRequestConfig<'_>,
        token_tracker: &TokenUsageTracker,
    ) -> Result<PreparedRequest> {
        let request = self.assemble(config.prompt, config.context, config.tools)?;
        Ok(self.prepare_request(request, config.microcompact, token_tracker))
    }

    pub(crate) fn assemble(
        &self,
        prompt: &PromptPlan,
        context: ContextBundle,
        mut tools: Vec<ToolDefinition>,
    ) -> Result<ModelRequest> {
        // These slots are intentionally carried through the bundle even before they affect request
        // encoding, so future workset/memory stages can plug in without changing loop signatures.
        let _structured_context_slots = (
            &context.workset,
            &context.memory,
            &context.diagnostics,
            &context.budget_state,
        );
        append_unique_tools(&mut tools, prompt.extra_tools.clone());
        Ok(ModelRequest {
            messages: self.compose_messages(prompt, &context.conversation),
            tools,
            system_prompt: prompt.render_system(),
        })
    }

    pub(crate) fn prepare_request(
        &self,
        request: ModelRequest,
        config: MicrocompactConfig<'_>,
        token_tracker: &TokenUsageTracker,
    ) -> PreparedRequest {
        let microcompact_result = apply_microcompact(
            &request.messages,
            config.capability_descriptors,
            config.tool_result_max_bytes,
            config.keep_recent_turns,
            effective_context_window(astrcode_runtime_llm::ModelLimits {
                context_window: config.model_context_window,
                max_output_tokens: 0,
            }),
        );
        self.prepared_from_microcompact(
            request,
            microcompact_result,
            config.model_context_window,
            config.compact_threshold_percent,
            token_tracker,
        )
    }

    pub(crate) fn rebuild_request_messages(
        &self,
        prompt: &PromptPlan,
        conversation: &ConversationView,
        capability_descriptors: &[CapabilityDescriptor],
        tool_result_max_bytes: usize,
        keep_recent_turns: usize,
        model_context_window: usize,
    ) -> Vec<LlmMessage> {
        apply_microcompact(
            &self.compose_messages(prompt, conversation),
            capability_descriptors,
            tool_result_max_bytes,
            keep_recent_turns,
            effective_context_window(astrcode_runtime_llm::ModelLimits {
                context_window: model_context_window,
                max_output_tokens: 0,
            }),
        )
        .messages
    }

    /// Refresh the model-visible request messages after compaction rebuilt the conversation view.
    ///
    /// Reactive compact is the only place that mutates the in-flight request after assembly.
    /// Keeping that mutation behind the assembler avoids leaking request encoding details back into
    /// `turn_runner`.
    pub(crate) fn rebuild_request_after_compaction(
        &self,
        request: &mut ModelRequest,
        prompt: &PromptPlan,
        conversation: &ConversationView,
        config: CompactionRebuildConfig<'_>,
    ) {
        request.messages = self.rebuild_request_messages(
            prompt,
            conversation,
            config.capability_descriptors,
            config.tool_result_max_bytes,
            config.keep_recent_turns,
            config.model_context_window,
        );
    }

    fn compose_messages(
        &self,
        prompt: &PromptPlan,
        conversation: &ConversationView,
    ) -> Vec<LlmMessage> {
        let mut messages = prompt.prepend_messages.clone();
        messages.extend(conversation.messages.iter().cloned());
        messages.extend(prompt.append_messages.clone());
        messages
    }

    fn prepared_from_microcompact(
        &self,
        mut request: ModelRequest,
        microcompact_result: MicrocompactResult,
        model_context_window: usize,
        compact_threshold_percent: u8,
        token_tracker: &TokenUsageTracker,
    ) -> PreparedRequest {
        request.messages = microcompact_result.messages;
        let prompt_snapshot = build_prompt_snapshot(
            token_tracker,
            &request.messages,
            request.system_prompt.as_deref(),
            astrcode_runtime_llm::ModelLimits {
                context_window: model_context_window,
                max_output_tokens: 0,
            },
            compact_threshold_percent,
        );
        PreparedRequest {
            request,
            prompt_snapshot,
            truncated_tool_results: microcompact_result.truncated_tool_results,
        }
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{ToolDefinition, UserMessageOrigin};
    use serde_json::json;

    use super::*;
    use crate::context_pipeline::{ContextBundle, ConversationView, TokenBudgetState};

    fn plan() -> PromptPlan {
        PromptPlan {
            prepend_messages: vec![LlmMessage::User {
                content: "prepend".to_string(),
                origin: UserMessageOrigin::User,
            }],
            append_messages: vec![LlmMessage::User {
                content: "append".to_string(),
                origin: UserMessageOrigin::User,
            }],
            extra_tools: vec![ToolDefinition {
                name: "extra".to_string(),
                description: "extra".to_string(),
                parameters: json!({"type":"object"}),
            }],
            ..PromptPlan::default()
        }
    }

    #[test]
    fn assemble_preserves_message_order_and_tool_union() {
        let request = RequestAssembler
            .assemble(
                &plan(),
                ContextBundle {
                    conversation: ConversationView::new(vec![LlmMessage::User {
                        content: "body".to_string(),
                        origin: UserMessageOrigin::User,
                    }]),
                    workset: Vec::new(),
                    memory: Vec::new(),
                    diagnostics: Vec::new(),
                    budget_state: TokenBudgetState,
                },
                vec![ToolDefinition {
                    name: "base".to_string(),
                    description: "base".to_string(),
                    parameters: json!({"type":"object"}),
                }],
            )
            .expect("request should assemble");

        assert_eq!(request.messages.len(), 3);
        assert_eq!(request.tools.len(), 2);
        assert!(matches!(
            &request.messages[0],
            LlmMessage::User { content, .. } if content == "prepend"
        ));
        assert!(matches!(
            &request.messages[1],
            LlmMessage::User { content, .. } if content == "body"
        ));
        assert!(matches!(
            &request.messages[2],
            LlmMessage::User { content, .. } if content == "append"
        ));
    }

    #[test]
    fn build_step_request_runs_microcompact_and_snapshot_in_request_order() {
        let prepared = RequestAssembler
            .build_step_request(
                StepRequestConfig {
                    prompt: &plan(),
                    context: ContextBundle {
                        conversation: ConversationView::new(vec![LlmMessage::User {
                            content: "body".to_string(),
                            origin: UserMessageOrigin::User,
                        }]),
                        workset: Vec::new(),
                        memory: Vec::new(),
                        diagnostics: Vec::new(),
                        budget_state: TokenBudgetState,
                    },
                    tools: vec![],
                    microcompact: MicrocompactConfig {
                        capability_descriptors: &[],
                        tool_result_max_bytes: 1024,
                        keep_recent_turns: 1,
                        model_context_window: 8192,
                        compact_threshold_percent: 80,
                    },
                },
                &TokenUsageTracker::default(),
            )
            .expect("prepared request should build");

        assert_eq!(prepared.request.messages.len(), 3);
        assert_eq!(prepared.prompt_snapshot.context_window, 8192);
        assert_eq!(prepared.truncated_tool_results, 0);
    }

    #[test]
    fn rebuild_request_after_compaction_refreshes_only_messages() {
        let mut request = RequestAssembler
            .assemble(
                &plan(),
                ContextBundle {
                    conversation: ConversationView::new(vec![LlmMessage::User {
                        content: "body".to_string(),
                        origin: UserMessageOrigin::User,
                    }]),
                    workset: Vec::new(),
                    memory: Vec::new(),
                    diagnostics: Vec::new(),
                    budget_state: TokenBudgetState,
                },
                vec![],
            )
            .expect("request should assemble");
        request.system_prompt = Some("system".to_string());

        RequestAssembler.rebuild_request_after_compaction(
            &mut request,
            &plan(),
            &ConversationView::new(vec![LlmMessage::User {
                content: "summary".to_string(),
                origin: UserMessageOrigin::CompactSummary,
            }]),
            CompactionRebuildConfig {
                capability_descriptors: &[],
                tool_result_max_bytes: 1024,
                keep_recent_turns: 1,
                model_context_window: 8192,
            },
        );

        assert_eq!(request.system_prompt.as_deref(), Some("system"));
        assert!(matches!(
            &request.messages[1],
            LlmMessage::User { content, .. } if content == "summary"
        ));
    }
}
