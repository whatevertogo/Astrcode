//! 桥接 `adapter-llm` 内部 trait 与 `core::ports::LlmProvider`。
//!
//! adapter-llm 有自己的 LlmProvider trait 和配套类型（LlmRequest/LlmOutput/LlmEvent 等），
//! 它们与 `core::ports` 中定义的端口类型结构相同但类型路径不同。
//! 本模块通过 trivial 转换让 OpenAiProvider / AnthropicProvider 同时满足 core 端口契约，
//! 使 server 组合根可以直接注入 adapter 实例而无需 Noop 替代。

use std::sync::Arc;

use astrcode_core::ports::{self, LlmFinishReason};
use async_trait::async_trait;

use crate::{
    FinishReason, LlmEvent, LlmOutput, LlmProvider, anthropic::AnthropicProvider,
    openai::OpenAiProvider,
};

// ── 类型转换辅助 ──────────────────────────────────────────

/// `adapter-llm::LlmRequest` → `core::ports::LlmRequest`
fn convert_request(req: ports::LlmRequest) -> crate::LlmRequest {
    crate::LlmRequest {
        messages: req.messages,
        tools: req.tools,
        cancel: req.cancel,
        system_prompt: req.system_prompt,
        system_prompt_blocks: req.system_prompt_blocks,
    }
}

/// `adapter-llm::LlmOutput` → `core::ports::LlmOutput`
fn convert_output(out: LlmOutput) -> ports::LlmOutput {
    ports::LlmOutput {
        content: out.content,
        tool_calls: out.tool_calls,
        reasoning: out.reasoning,
        usage: out.usage.map(convert_usage),
        finish_reason: convert_finish_reason(out.finish_reason),
    }
}

fn convert_usage(u: crate::LlmUsage) -> ports::LlmUsage {
    ports::LlmUsage {
        input_tokens: u.input_tokens,
        output_tokens: u.output_tokens,
        cache_creation_input_tokens: u.cache_creation_input_tokens,
        cache_read_input_tokens: u.cache_read_input_tokens,
    }
}

fn convert_finish_reason(r: FinishReason) -> LlmFinishReason {
    match r {
        FinishReason::Stop => LlmFinishReason::Stop,
        FinishReason::MaxTokens => LlmFinishReason::MaxTokens,
        FinishReason::ToolCalls => LlmFinishReason::ToolCalls,
        FinishReason::Other(s) => LlmFinishReason::Other(s),
    }
}

/// `adapter-llm::LlmEvent` → `core::ports::LlmEvent`
fn convert_event(event: LlmEvent) -> ports::LlmEvent {
    match event {
        LlmEvent::TextDelta(t) => ports::LlmEvent::TextDelta(t),
        LlmEvent::ThinkingDelta(t) => ports::LlmEvent::ThinkingDelta(t),
        LlmEvent::ThinkingSignature(s) => ports::LlmEvent::ThinkingSignature(s),
        LlmEvent::ToolCallDelta {
            index,
            id,
            name,
            arguments_delta,
        } => ports::LlmEvent::ToolCallDelta {
            index,
            id,
            name,
            arguments_delta,
        },
    }
}

/// 将 core 端的 sink 包装为 adapter 内部的 EventSink。
///
/// adapter 内部 generate() 会产出 `adapter::LlmEvent`，
/// 我们把每个 event 转为 `core::ports::LlmEvent` 后转发给外部 sink。
fn wrap_sink(core_sink: ports::LlmEventSink) -> crate::EventSink {
    Arc::new(move |event: LlmEvent| {
        core_sink(convert_event(event));
    })
}

fn convert_limits(limits: crate::ModelLimits) -> ports::ModelLimits {
    ports::ModelLimits {
        context_window: limits.context_window,
        max_output_tokens: limits.max_output_tokens,
    }
}

// ── core::ports::LlmProvider 实现 ─────────────────────────

#[async_trait]
impl ports::LlmProvider for OpenAiProvider {
    async fn generate(
        &self,
        request: ports::LlmRequest,
        sink: Option<ports::LlmEventSink>,
    ) -> astrcode_core::Result<ports::LlmOutput> {
        let internal_request = convert_request(request);
        let internal_sink = sink.map(wrap_sink);
        let output = LlmProvider::generate(self, internal_request, internal_sink).await?;
        Ok(convert_output(output))
    }

    fn model_limits(&self) -> ports::ModelLimits {
        convert_limits(LlmProvider::model_limits(self))
    }

    fn supports_cache_metrics(&self) -> bool {
        LlmProvider::supports_cache_metrics(self)
    }
}

#[async_trait]
impl ports::LlmProvider for AnthropicProvider {
    async fn generate(
        &self,
        request: ports::LlmRequest,
        sink: Option<ports::LlmEventSink>,
    ) -> astrcode_core::Result<ports::LlmOutput> {
        let internal_request = convert_request(request);
        let internal_sink = sink.map(wrap_sink);
        let output = LlmProvider::generate(self, internal_request, internal_sink).await?;
        Ok(convert_output(output))
    }

    fn model_limits(&self) -> ports::ModelLimits {
        convert_limits(LlmProvider::model_limits(self))
    }

    fn supports_cache_metrics(&self) -> bool {
        LlmProvider::supports_cache_metrics(self)
    }
}
