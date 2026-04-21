//! # Anthropic Messages API 提供者
//!
//! 实现了 [`LlmProvider`] trait，对接 Anthropic Claude 系列模型。
//!
//! ## 协议特性
//!
//! - **Extended Thinking**: 自动为 Claude 模型启用深度推理模式（`thinking` 配置）， 预算 token 设为
//!   `max_tokens` 的 75%，保留至少 25% 给实际输出
//! - **Prompt Caching**: 优先对分层 system blocks 放置 `ephemeral` breakpoint，并在消息尾部保留
//!   一个缓存边界，复用 KV cache
//! - **SSE 流式解析**: Anthropic 使用多行 SSE 块格式（`event: ...\ndata: {...}\n\n`）， 与 OpenAI
//!   的单行 `data: {...}` 不同，因此有独立的解析逻辑
//! - **内容块模型**: Anthropic 响应由多种内容块组成（text / tool_use / thinking）， 使用
//!   `Vec<Value>` 灵活处理未知或新增的块类型
//!
//! ## 流式事件分派
//!
//! Anthropic SSE 事件类型：
//! - `content_block_start`: 新内容块开始（文本或工具调用）
//! - `content_block_delta`: 增量内容（text_delta / thinking_delta / signature_delta /
//!   input_json_delta）
//! - `message_stop`: 流结束信号
//! - `message_start / message_delta`: 提取 usage / stop_reason 等元数据
//! - `content_block_stop / ping`: 元数据事件，静默忽略

pub(crate) mod dto;
mod provider;
mod request;
mod response;
mod stream;

pub use provider::AnthropicProvider;
