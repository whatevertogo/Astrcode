# ADR-0007: Use Layered Prompt Construction for KV Cache Optimization

- Status: Accepted
- Date: 2026-04-03

## Context

当前 prompt 构建会频繁整体重建 system prompt。即使只有工具列表或技能摘要变化，也会破坏前缀稳定性，降低 LLM KV cache 命中率，增加延迟与成本。为了提升缓存复用，需要把 prompt 中”长期稳定”和”高频变化”的内容明确分层。

## Decision

当引入 KV-cache-aware prompt 构建时，采用分层 prompt 结构，而不是继续把所有内容视为同一层。

- 稳定层承载几乎不变的内容，例如 identity 和 environment。
- 半稳定层承载按配置或规则变化的内容，例如用户规则、项目规则和扩展指令。
- 动态层承载高频变化内容，例如工具列表、技能摘要和工作流示例。
- 层的排列顺序必须保证稳定前缀尽量固定，把频繁变化内容放在后部，以提升 KV cache 复用率。
- 在该方案真正接入运行时前，生产路径继续使用现有 prompt 组装方式。

## Consequences

- 未来实现需要围绕”前缀稳定性”和”明确失效边界”设计缓存策略。
- prompt 组装会比单层方案更复杂，需要维护分层失效规则。
- `LayeredPromptBuilder` 已在 `runtime-prompt` 中实现，但尚未接入 `agent_loop`；在正式接入前，该 ADR 不改变当前生产行为。
