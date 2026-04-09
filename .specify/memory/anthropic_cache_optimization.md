---
name: Anthropic 缓存优化三阶段
description: 003 分支附带 — Phase 1 缓存可见性改进、Phase 2 缓存策略优化（消息缓存深度3）、Phase 3 缓存失效检测机制（CacheTracker）
type: project
---

# Anthropic 缓存优化三阶段

**Why:** Anthropic API 的 KV cache 对长对话成本和延迟影响大，但缺乏缓存命中/失效的可见性和主动优化。三阶段改进提升缓存利用率。

**How to apply:** 所有涉及 Anthropic provider 的缓存行为由 `CacheTracker` 跟踪，消息缓存深度已从 1 提升到 3。未来调优缓存策略时可查看 tracker 日志。

## Phase 1: 缓存可见性改进

增加缓存命中/未命中的日志输出，使开发者能观察实际缓存行为。

## Phase 2: 优化 Anthropic 缓存策略

- 消息缓存深度从 1 提升到 3（`enable_message_caching(&mut anthropic_messages, 3)`）
- 提高长对话的缓存命中率
- 实现位置: `crates/runtime-llm/src/anthropic.rs`

## Phase 3: 缓存失效检测机制

新增 `CacheTracker`（`crates/runtime-llm/src/cache_tracker.rs`，约 254 行），负责：

- 跟踪 system prompt 变化导致的缓存失效
- 检测 tool 集合变化对缓存的影响
- 记录失效原因，便于诊断缓存性能问题
- 集成到 `AnthropicProvider` 中，在每次请求前检测缓存失效

关键实现：
- `CacheTracker` 使用 `Arc<Mutex<CacheTracker>>` 包装，线程安全
- 在 `LlmProvider::generate` 中，构建请求前先检测并记录失效原因
- 对比 `SystemPromptLayer` 变化和 tool 名称列表检测失效

## 实现文件

- `crates/runtime-llm/src/cache_tracker.rs` — CacheTracker 实现（新增文件）
- `crates/runtime-llm/src/anthropic.rs` — AnthropicProvider 集成 CacheTracker、消息缓存深度调整
- `crates/runtime-llm/src/lib.rs` — 模块导出
