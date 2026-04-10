# ADR-0007: Use Layered Prompt Construction for KV Cache Optimization

- Status: Accepted
- Date: 2026-04-03

## Context

简单的 prompt 组装把所有内容当成同一层，导致系统提示词前缀可变性高。即使只有工具列表、skill 概要或临时上下文变化，也会破坏 LLM 的 KV cache 命中率，增加延迟与成本。

## Decision

在 runtime prompt 构建中采用分层结构，把长期稳定内容和高频变化内容明确隔离。

- `Stable` 层承载几乎不变的内容，例如 agent identity、环境说明和顶层 policy 约束。
- `SemiStable` 层承载按配置或 profile 变化的内容，例如用户规则、项目规则和 runtime 指令。
- `Inherited` 层承载跨子任务/子会话传递但不频繁变化的上下文，例如 parent delivery 声明或子任务继承摘要。
- `Dynamic` 层承载高频变化内容，例如工具列表、当前工具调用描述、prompt declarations 和最近消息上下文。
- prompt layers 的排列顺序应尽量保证稳定前缀在前面，并把频繁变化内容放在后部，以提升 KV cache 复用率。
- `astrcode_runtime_prompt::LayeredPromptBuilder` 已在 `crates/runtime-prompt` 中实现，并由 `crates/runtime-agent-loop::PromptRuntime` 实际接入运行时。

## Consequences

- prompt 组装更适合缓存命中与层级失效控制。
- runtime prompt 逻辑会更复杂，需要维护每层的失效边界与 cache_boundary 语义。
- `LayeredPromptBuilder` 的层级设计已成为当前运行时 prompt 组装的一部分，而不是只是实验性提案。
