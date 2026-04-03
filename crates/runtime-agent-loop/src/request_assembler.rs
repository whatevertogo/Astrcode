//! # Request Assembler（请求装配器）
//!
//! ## 职责
//!
//! 执行 Prompt Plan 和 Context Bundle 之间的最终请求编码步骤。
//! Prompt Planner 决定指令支架，Context Pipeline 决定模型可见对话，
//! 本模块将两者合并为完整的 `ModelRequest`，并生成 prompt 快照用于指标上报。
//!
//! ## 在 Turn 流程中的作用
//!
//! - **调用时机**：每个 step 中，`build_plan()` 和 `decide_context_strategy()` 之后
//! - **输入**：`StepRequestConfig`（prompt plan + context bundle + 工具列表 + 模型窗口）
//!              + `TokenUsageTracker`（当前 token 使用统计）
//! - **输出**：`PreparedRequest`（`ModelRequest` + `PromptTokenSnapshot` + 截断计数）
//!
//! ## 依赖和协作
//!
//! - **使用** `build_prompt_snapshot()` 生成 token 估算快照（system tokens + message tokens）
//! - **使用** `append_unique_tools()` 将工具定义去重后注入到 `ModelRequest.tools`
//! - **使用** `ContextBundle.conversation` 获取裁剪后的模型可见消息列表
//! - **使用** `PromptPlan` 获取系统提示词和可选规划结果
//! - **被调用方**：`turn_runner` 在 LLM 调用前调用 `build_step_request()`
//! - **输出给**：`llm_cycle::generate_response()` 消费 `ModelRequest`
//!                 `on_event(StorageEvent::PromptMetrics {...})` 消费 `PromptTokenSnapshot`
//!
//! ## 关键设计
//!
//! - `StepRequestConfig` 结构体将 5 个相关参数打包为一个，避免函数签名膨胀
//! - `PromptTokenSnapshot` 携带 context_tokens / threshold_tokens / effective_window 等字段，
//!   供 `CompactionRuntime` 决策时使用
//! - `truncated_tool_results` 返回被 microcompact 裁剪的工具结果数量，上报给前端指标

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
