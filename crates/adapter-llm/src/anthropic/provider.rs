use std::{
    fmt,
    sync::{Arc, Mutex},
};

use astrcode_core::{
    AstrError, CancelToken, LlmMessage, Result, SystemPromptBlock, ToolDefinition,
};
use async_trait::async_trait;
use futures_util::StreamExt;
use log::{debug, warn};
use tokio::select;

use super::{
    dto::{AnthropicCacheControl, AnthropicRequest, AnthropicResponse, AnthropicUsage},
    request::{
        ANTHROPIC_CACHE_BREAKPOINT_LIMIT, MessageBuildOptions, enable_message_caching,
        is_official_anthropic_api_url, summarize_request_for_diagnostics,
        thinking_config_for_model, to_anthropic_messages, to_anthropic_system, to_anthropic_tools,
    },
    response::response_to_output,
    stream::{consume_sse_text_chunk, flush_sse_buffer},
};
use crate::{
    EventSink, FinishReason, LlmAccumulator, LlmClientConfig, LlmOutput, LlmProvider, LlmRequest,
    ModelLimits, Utf8StreamDecoder, build_http_client, cache_tracker::CacheTracker,
    classify_http_error, is_retryable_status, wait_retry_delay,
};

const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Anthropic Claude API 提供者实现。
///
/// 封装了 HTTP 客户端、API 密钥和模型配置，提供统一的 [`LlmProvider`] 接口。
///
/// ## 设计要点
///
/// - HTTP 客户端在构造时创建，使用共享的超时策略（连接 10s / 读取 90s）
/// - `limits.max_output_tokens` 同时控制请求体的上限和 extended thinking 的预算计算
/// - Debug 实现会隐藏 API 密钥（显示为 `<redacted>`）
#[derive(Clone)]
pub struct AnthropicProvider {
    client: reqwest::Client,
    client_config: LlmClientConfig,
    messages_api_url: String,
    api_key: String,
    model: String,
    /// 运行时已解析好的模型 limits。
    ///
    /// Anthropic 的上下文窗口来自 Models API，不应该继续在 provider 内写死。
    limits: ModelLimits,
    /// 缓存失效检测跟踪器
    cache_tracker: Arc<Mutex<CacheTracker>>,
}

impl fmt::Debug for AnthropicProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("client", &self.client)
            .field("messages_api_url", &self.messages_api_url)
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .field("limits", &self.limits)
            .field("client_config", &self.client_config)
            .field("cache_tracker", &"<internal>")
            .finish()
    }
}

impl AnthropicProvider {
    /// 创建新的 Anthropic 提供者实例。
    ///
    /// `limits.max_output_tokens` 同时用于：
    /// 1. 请求体中的 `max_tokens` 字段（输出上限）
    /// 2. Extended thinking 预算计算（75% 的 max_tokens）
    pub fn new(
        messages_api_url: String,
        api_key: String,
        model: String,
        limits: ModelLimits,
        client_config: LlmClientConfig,
    ) -> Result<Self> {
        Ok(Self {
            client: build_http_client(client_config)?,
            client_config,
            messages_api_url,
            api_key,
            model,
            limits,
            cache_tracker: Arc::new(Mutex::new(CacheTracker::new())),
        })
    }

    /// 构建 Anthropic Messages API 请求体。
    ///
    /// - 将 `LlmMessage` 转换为 Anthropic 格式的内容块数组
    /// - 对分层 system blocks 和消息尾部启用 prompt caching（KV cache 复用）
    /// - 如果启用了工具，附加工具定义
    /// - 根据模型名称和 max_tokens 自动配置 extended thinking
    pub(super) fn build_request(
        &self,
        messages: &[LlmMessage],
        tools: &[ToolDefinition],
        system_prompt: Option<&str>,
        system_prompt_blocks: &[SystemPromptBlock],
        max_output_tokens_override: Option<usize>,
        stream: bool,
    ) -> AnthropicRequest {
        let effective_max_output_tokens = max_output_tokens_override
            .unwrap_or(self.limits.max_output_tokens)
            .min(self.limits.max_output_tokens);
        let use_official_endpoint = is_official_anthropic_api_url(&self.messages_api_url);
        let use_automatic_cache = use_official_endpoint;
        let mut remaining_cache_breakpoints = ANTHROPIC_CACHE_BREAKPOINT_LIMIT;
        let request_cache_control = if use_automatic_cache {
            remaining_cache_breakpoints = remaining_cache_breakpoints.saturating_sub(1);
            Some(AnthropicCacheControl::ephemeral())
        } else {
            None
        };

        let mut anthropic_messages = to_anthropic_messages(
            messages,
            MessageBuildOptions {
                include_reasoning_blocks: use_official_endpoint,
            },
        );
        let tools = if tools.is_empty() {
            None
        } else {
            Some(to_anthropic_tools(tools, &mut remaining_cache_breakpoints))
        };
        let system = to_anthropic_system(
            system_prompt,
            system_prompt_blocks,
            &mut remaining_cache_breakpoints,
        );

        if !use_automatic_cache {
            enable_message_caching(&mut anthropic_messages, remaining_cache_breakpoints);
        }

        AnthropicRequest {
            model: self.model.clone(),
            max_tokens: effective_max_output_tokens.min(u32::MAX as usize) as u32,
            cache_control: request_cache_control,
            messages: anthropic_messages,
            system,
            tools,
            stream: stream.then_some(true),
            // Why: 第三方 Anthropic 兼容网关常见只支持基础 messages 子集；
            // 在非官方 endpoint 下关闭 `thinking` 字段，避免触发参数校验失败。
            thinking: if use_official_endpoint {
                thinking_config_for_model(
                    &self.model,
                    effective_max_output_tokens.min(u32::MAX as usize) as u32,
                )
            } else {
                None
            },
        }
    }

    async fn send_request(
        &self,
        request: &AnthropicRequest,
        cancel: CancelToken,
    ) -> Result<reqwest::Response> {
        // 调试日志：打印请求信息（不暴露完整 API Key）
        let api_key_preview = if self.api_key.len() > 8 {
            format!(
                "{}...{}",
                &self.api_key[..4],
                &self.api_key[self.api_key.len() - 4..]
            )
        } else {
            "****".to_string()
        };
        debug!(
            "Anthropic request: url={}, api_key_preview={}, model={}",
            self.messages_api_url, api_key_preview, self.model
        );
        if !is_official_anthropic_api_url(&self.messages_api_url) {
            debug!(
                "Anthropic-compatible request summary: {}",
                summarize_request_for_diagnostics(request)
            );
        }

        for attempt in 0..=self.client_config.max_retries {
            let send_future = self
                .client
                .post(&self.messages_api_url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .json(request)
                .send();

            let response = select! {
                _ = crate::cancelled(cancel.clone()) => {
                    return Err(AstrError::LlmInterrupted);
                }
                result = send_future => result.map_err(|e| AstrError::http("failed to call anthropic endpoint", e))
            };

            match response {
                Ok(response) => {
                    let status = response.status();
                    if status == reqwest::StatusCode::UNAUTHORIZED {
                        // 读取响应体以便调试
                        let body = response.text().await.unwrap_or_default();
                        warn!(
                            "Anthropic 401 Unauthorized: url={}, api_key_preview={}, response={}",
                            self.messages_api_url,
                            if self.api_key.len() > 8 {
                                format!(
                                    "{}...{}",
                                    &self.api_key[..4],
                                    &self.api_key[self.api_key.len() - 4..]
                                )
                            } else {
                                "****".to_string()
                            },
                            body
                        );
                        return Err(AstrError::InvalidApiKey("Anthropic".to_string()));
                    }
                    if status.is_success() {
                        return Ok(response);
                    }

                    let body = response.text().await.unwrap_or_default();
                    if is_retryable_status(status) && attempt < self.client_config.max_retries {
                        wait_retry_delay(
                            attempt,
                            cancel.clone(),
                            self.client_config.retry_base_delay,
                        )
                        .await?;
                        continue;
                    }

                    if status.is_client_error()
                        && !is_official_anthropic_api_url(&self.messages_api_url)
                    {
                        warn!(
                            "Anthropic-compatible request rejected: url={}, status={}, \
                             request_summary={}, response={}",
                            self.messages_api_url,
                            status.as_u16(),
                            summarize_request_for_diagnostics(request),
                            body
                        );
                    }

                    // 使用结构化错误分类 (P4.3)
                    return Err(classify_http_error(status.as_u16(), &body).into());
                },
                Err(error) => {
                    if error.is_retryable() && attempt < self.client_config.max_retries {
                        wait_retry_delay(
                            attempt,
                            cancel.clone(),
                            self.client_config.retry_base_delay,
                        )
                        .await?;
                        continue;
                    }
                    return Err(error);
                },
            }
        }

        // 所有路径都会通过 return 退出循环；若到达此处说明逻辑有误，
        // 返回 Internal 而非 panic 以保证运行时安全
        Err(AstrError::Internal(
            "retry loop should have returned on all paths".into(),
        ))
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn supports_cache_metrics(&self) -> bool {
        true
    }

    async fn generate(&self, request: LlmRequest, sink: Option<EventSink>) -> Result<LlmOutput> {
        let cancel = request.cancel;

        // 检测缓存失效并记录原因
        let system_prompt_text = request
            .prompt_cache_hints
            .as_ref()
            .map(cacheable_prefix_cache_key)
            .unwrap_or_else(|| request.system_prompt.clone().unwrap_or_default());
        let tool_names: Vec<String> = request.tools.iter().map(|t| t.name.clone()).collect();

        if let Ok(mut tracker) = self.cache_tracker.lock() {
            let break_reasons = tracker.check_and_update(
                &system_prompt_text,
                &tool_names,
                &self.model,
                "anthropic",
            );

            if !break_reasons.is_empty() {
                debug!(
                    "[CACHE] Cache break detected: {:?}, unchanged_layers={:?}",
                    break_reasons,
                    request
                        .prompt_cache_hints
                        .as_ref()
                        .map(|hints| hints.unchanged_layers.as_slice())
                        .unwrap_or(&[])
                );
            }
        }

        let body = self.build_request(
            &request.messages,
            &request.tools,
            request.system_prompt.as_deref(),
            &request.system_prompt_blocks,
            request.max_output_tokens_override,
            sink.is_some(),
        );
        let response = self.send_request(&body, cancel.clone()).await?;

        match sink {
            None => {
                let payload: AnthropicResponse = response
                    .json()
                    .await
                    .map_err(|e| AstrError::http("failed to parse anthropic response", e))?;
                Ok(response_to_output(payload))
            },
            Some(sink) => {
                let mut stream = response.bytes_stream();
                let mut sse_buffer = String::new();
                let mut utf8_decoder = Utf8StreamDecoder::default();
                let mut accumulator = LlmAccumulator::default();
                // 流式路径下从 message_delta 的 stop_reason 提取 (P4.2)
                let mut stream_stop_reason: Option<String> = None;
                let mut stream_usage = AnthropicUsage::default();

                loop {
                    let next_item = select! {
                        _ = crate::cancelled(cancel.clone()) => {
                            return Err(AstrError::LlmInterrupted);
                        }
                        item = stream.next() => item,
                    };

                    let Some(item) = next_item else {
                        break;
                    };

                    let bytes = item.map_err(|e| {
                        AstrError::http("failed to read anthropic response stream", e)
                    })?;
                    let Some(chunk_text) = utf8_decoder
                        .push(&bytes, "anthropic response stream was not valid utf-8")?
                    else {
                        continue;
                    };

                    if consume_sse_text_chunk(
                        &chunk_text,
                        &mut sse_buffer,
                        &mut accumulator,
                        &sink,
                        &mut stream_stop_reason,
                        &mut stream_usage,
                    )? {
                        let mut output = accumulator.finish();
                        // 优先使用 API 返回的 stop_reason，否则使用推断值
                        if let Some(reason) = stream_stop_reason.as_deref() {
                            output.finish_reason = FinishReason::from_api_value(reason);
                        }
                        output.usage = stream_usage.into_llm_usage();

                        // 记录流式响应的缓存状态
                        if let Some(ref u) = output.usage {
                            let input = u.input_tokens;
                            let cache_read = u.cache_read_input_tokens;
                            let cache_creation = u.cache_creation_input_tokens;
                            let total_prompt_tokens = input.saturating_add(cache_read);

                            if cache_read == 0 && cache_creation > 0 {
                                debug!(
                                    "Cache miss (streaming): writing {} tokens to cache (total \
                                     prompt: {}, uncached input: {})",
                                    cache_creation, total_prompt_tokens, input
                                );
                            } else if cache_read > 0 {
                                let hit_rate =
                                    (cache_read as f32 / total_prompt_tokens as f32) * 100.0;
                                debug!(
                                    "Cache hit (streaming): {:.1}% ({} / {} prompt tokens, \
                                     creation: {}, uncached input: {})",
                                    hit_rate,
                                    cache_read,
                                    total_prompt_tokens,
                                    cache_creation,
                                    input
                                );
                            } else {
                                debug!(
                                    "Cache disabled or unavailable (streaming, total prompt: {} \
                                     tokens)",
                                    total_prompt_tokens
                                );
                            }
                        }

                        return Ok(output);
                    }
                }

                if let Some(tail_text) =
                    utf8_decoder.finish("anthropic response stream was not valid utf-8")?
                {
                    let done = consume_sse_text_chunk(
                        &tail_text,
                        &mut sse_buffer,
                        &mut accumulator,
                        &sink,
                        &mut stream_stop_reason,
                        &mut stream_usage,
                    )?;
                    if done {
                        let mut output = accumulator.finish();
                        if let Some(reason) = stream_stop_reason.as_deref() {
                            output.finish_reason = FinishReason::from_api_value(reason);
                        }
                        output.usage = stream_usage.into_llm_usage();
                        return Ok(output);
                    }
                }

                flush_sse_buffer(
                    &mut sse_buffer,
                    &mut accumulator,
                    &sink,
                    &mut stream_stop_reason,
                    &mut stream_usage,
                )?;
                let mut output = accumulator.finish();
                if let Some(reason) = stream_stop_reason.as_deref() {
                    output.finish_reason = FinishReason::from_api_value(reason);
                }
                output.usage = stream_usage.into_llm_usage();
                Ok(output)
            },
        }
    }

    fn model_limits(&self) -> ModelLimits {
        self.limits
    }
}

fn cacheable_prefix_cache_key(hints: &astrcode_core::PromptCacheHints) -> String {
    [
        hints.layer_fingerprints.stable.as_deref(),
        hints.layer_fingerprints.semi_stable.as_deref(),
        hints.layer_fingerprints.inherited.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("|")
}

#[cfg(test)]
mod tests {
    use super::AnthropicProvider;
    use crate::{LlmClientConfig, LlmProvider, ModelLimits};

    #[test]
    fn provider_keeps_custom_messages_api_url() {
        let provider = AnthropicProvider::new(
            "https://gateway.example.com/anthropic/v1/messages".to_string(),
            "sk-ant-test".to_string(),
            "claude-sonnet-4-5".to_string(),
            ModelLimits {
                context_window: 200_000,
                max_output_tokens: 8096,
            },
            LlmClientConfig::default(),
        )
        .expect("provider should build");

        assert_eq!(
            provider.messages_api_url,
            "https://gateway.example.com/anthropic/v1/messages"
        );
    }

    #[test]
    fn anthropic_provider_reports_cache_metrics_support() {
        let provider = AnthropicProvider::new(
            "https://api.anthropic.com/v1/messages".to_string(),
            "sk-ant-test".to_string(),
            "claude-sonnet-4-5".to_string(),
            ModelLimits {
                context_window: 200_000,
                max_output_tokens: 8096,
            },
            LlmClientConfig::default(),
        )
        .expect("provider should build");

        assert!(provider.supports_cache_metrics());
    }
}
