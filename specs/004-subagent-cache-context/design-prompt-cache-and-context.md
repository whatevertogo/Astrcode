# Design: Prompt Cache And Context

## Goals

- 让子智能体的首条任务消息只表达任务目标与直接上下文。
- 把父背景拆成可独立更新、可独立缓存的 prompt 继承块。
- 复用 `runtime-prompt` 现有 fingerprint 与 LayerCache，而不是再造 key 规则。
- 让缓存收益和误命中都能通过 telemetry 验证。

## Non-Goals

- 不在本 feature 中设计新的通用摘要模型。
- 不为 recent tail 引入额外推理调用。
- 不把 inherited context 写回 durable transcript。

## Context Split

`resolve_context_snapshot()` 的目标输出应显式分成两部分：

1. `task_payload`
2. `inherited_context`

### `task_payload`

- 只包含任务目标和必要的直接上下文
- 会进入 child 的任务消息流
- 不拼接父 compact summary 或 recent tail 全文

### `inherited_context`

- 由 `compact_summary` 与 `recent_tail` 两个 `InheritedContextBlock` 组成
- 通过 `PromptDeclaration` 注入 system blocks
- 注入到 `LayeredPromptBuilder` 的 `Inherited` 层，而不是 `Dynamic` 层
- 在 prompt 层拥有独立的 fingerprint 和缓存边界
- 当前实现里这些 block 只存在于 prompt metadata / runtime declarations，不会被翻译成 durable `UserMessage`

## Inherited Blocks

**层级约束**

- `Inherited` 层位于 `SemiStable` 层之后、`Dynamic` 层之前
- `compact_summary` 与 `recent_tail` 必须各自形成独立 cache boundary
- 实现时不得把 inherited blocks 回落到 `Dynamic` 层，否则会破坏稳定缓存收益

### `compact_summary`

- 更新频率低
- 优先作为稳定层参与缓存复用
- 必须来源于父会话已有的 durable / projected 摘要材料

### `recent_tail`

- 更新频率高
- 只保留关键用户输入、关键结论、必要工具摘要
- 必须经过确定性筛选与预算裁剪

## Deterministic Tail Filtering

默认规则按如下顺序执行：

1. 保留最近关键用户输入
2. 保留最近关键 assistant 结论
3. 将大型工具输出压缩成短摘要
4. 删除重复噪音和低价值活动
5. 若仍超预算，按角色优先级继续裁剪

**Constraint**

- 不为了整理 recent tail 单独触发新的推理回合。
- internal origin 的 `UserMessage`、相邻重复噪音和超长工具原文不会再直接进入 inherited tail。

## Shared Cache Boundary

缓存复用不由 `runtime-execution` 手写 key 判断，而是依赖 `runtime-prompt` 的 fingerprint 与 LayerCache：

- working dir、profile、工具集合、rules、prompt declarations、skills、工具元数据、builder version 等输入进入 fingerprint
- `compact_summary` 与 `recent_tail` 分别形成独立缓存段
- 任一影响 prompt 结果的输入变化都必须导致对应缓存段失效
- 父交付 wake turn 使用的 runtime-only declaration 不进入 durable transcript，也不会污染 child task payload 的缓存边界

## Metrics And Observability

### Supported Providers

- 必须产出 `PromptMetrics.cache_creation_input_tokens`
- 以该字段验证后续 child 启动相比首次下降至少 70%

### Unsupported Providers

- 必须在本 feature 交付前补齐等价缓存可观测指标
- 在补齐前，不纳入 `SC-003` 验收范围

### Logging

至少需要记录以下信息：

- fingerprint 命中或失效
- 失效原因类别
- inherited blocks 的预算裁剪结果
- provider 是否支持缓存指标

## Implemented Validation Focus

- child 首条任务消息应只剩 task payload，即使存在 inherited blocks 也不能回落到消息流。
- `PromptMetrics.cache_creation_input_tokens` 与 reuse hit/miss 应能区分 inherited segment 命中和失效。
- recent tail 的确定性裁剪必须在同一输入下稳定重现，避免 prompt cache 因顺序抖动失效。

## Rejected Alternatives

- 继续把 summary/tail 拼进首条任务消息：会持续污染消息语义和缓存边界。
- 在 spec 里枚举固定缓存 key 字段并自行实现哈希：会与 `runtime-prompt` 双轨分裂。
- 把交付详情或 inherited context 写成 durable 消息：会把 prompt 结构问题升级成 durable 真相污染。
