//！请求程序集边界。
//！
//！及时规划决定指令支架，上下文规划决定模型可见
//！对话，该模块执行两者之间的最终请求编码步骤

use astrcode_core::{LlmMessage, ModelRequest, Result, ToolDefinition};
use astrcode_runtime_prompt::{append_unique_tools, PromptPlan};

use crate::context_pipeline::{ContextBundle, ConversationView};
use crate::context_window::{build_prompt_snapshot, PromptTokenSnapshot, TokenUsageTracker};

pub(crate) struct RequestAssembler;

/// Prepared request plus request-shape diagnostics needed by the loop.
pub(crate) struct PreparedRequest {
    pub request: ModelRequest,
    pub prompt_snapshot: PromptTokenSnapshot,
    pub truncated_tool_results: usize,
}

/// Inputs needed to build and prepare a step request in one place.
pub(crate) struct StepRequestConfig<'a> {
    pub prompt: &'a PromptPlan,
    pub context: ContextBundle,
    pub tools: Vec<ToolDefinition>,
    pub model_context_window: usize,
    pub compact_threshold_percent: u8,
}

impl RequestAssembler {
    /// 组装最终请求结构并立即运行请求级预处理。
    ///
    /// 将这两个步骤保持在一起可以防止 `turn_runner` 拥有消息组合、工具联合、微压缩和提示快照生成的确切顺序
    /// 这些都是紧密耦合的细节，未来可能会频繁调整。现在 request assembler 只负责最终编码
    /// 和基于编码结果的快照，不再偷偷修改 conversation。
    pub(crate) fn build_step_request(
        &self,
        config: StepRequestConfig<'_>,
        token_tracker: &TokenUsageTracker,
    ) -> Result<PreparedRequest> {
        let truncated_tool_results = config.context.truncated_tool_results;
        let request = self.assemble(config.prompt, config.context, config.tools)?;
        Ok(self.prepare_request(
            request,
            config.model_context_window,
            config.compact_threshold_percent,
            truncated_tool_results,
            token_tracker,
        ))
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
        model_context_window: usize,
        compact_threshold_percent: u8,
        truncated_tool_results: usize,
        token_tracker: &TokenUsageTracker,
    ) -> PreparedRequest {
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
            truncated_tool_results,
        }
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
                    truncated_tool_results: 0,
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
    fn build_step_request_uses_pipeline_trimmed_context_and_builds_snapshot() {
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
                        truncated_tool_results: 2,
                    },
                    tools: vec![],
                    model_context_window: 8192,
                    compact_threshold_percent: 80,
                },
                &TokenUsageTracker::default(),
            )
            .expect("prepared request should build");

        assert_eq!(prepared.request.messages.len(), 3);
        assert_eq!(prepared.prompt_snapshot.context_window, 8192);
        assert_eq!(prepared.truncated_tool_results, 2);
    }
}
