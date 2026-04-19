## Why

现在的 `session-runtime` 虽然已经能实时广播 LLM 文本 delta，但 tool 调用仍然要等完整 `LlmOutput` 返回后才进入 `tool_cycle`。这意味着模型已经在流式给出 `tool_call` 相关增量时，系统仍然空转等待，失去了 Claude Code 那种“模型边产出可执行的 tool_use，系统边开始执行安全工具”的重要效率优势。

这条能力的价值不在于照搬 `query.ts` 的大循环，而在于借鉴 Claude Code `StreamingToolExecutor` 背后的核心思想，把“流式感知 -> 可执行候选组装 -> 安全调度 -> 有序落盘”嵌入现有 Rust 架构。只要边界处理对，Astrcode 完全可以在保持模块化的同时拿到这项收益。

## What Changes

- 让 `session-runtime` 在接收 LLM `ToolCallDelta` 时先组装 step-local 的可执行工具候选，而不是只把 delta 当成 live UI 信号丢弃
- 支持对“参数已经闭合、调用 identity 稳定、能力声明为 concurrency-safe”的工具调用提前开始执行，减少等待完整 assistant 输出的空档
- 保持副作用工具的保守策略：写操作或未完成参数的调用仍然等待完整 `LlmOutput`
- 为“提前执行但延后持久化排序”的场景定义稳定规则，并补上 discard / fallback 语义，避免破坏当前 event log 的可回放性
- 增加与流式 tool 调度相关的 observability / diagnostics，帮助判断是否真的带来重叠收益

## Capabilities

### New Capabilities

- 无

### Modified Capabilities

- `turn-orchestration`: turn loop 需要在 LLM 流式阶段组装可执行工具候选并调度符合条件的工具调用
- `runtime-observability-pipeline`: 运行时 observability 需要覆盖流式 tool 调度与 LLM/tool 重叠执行诊断

## Impact

- 受影响 crate：`crates/session-runtime`、`crates/core`
- 重点模块：`crates/session-runtime/src/turn/llm_cycle.rs`、`crates/session-runtime/src/turn/tool_cycle.rs`、`crates/session-runtime/src/turn/runner/step.rs`、相关 event / summary 模块
- 用户可见影响：只读工具密集场景下，回答与工具结果会更早开始推进，整体等待时间下降
- 开发者可见影响：tool 调度从“后置批处理”升级为“流式候选组装 + 保守执行”，需要更明确的生命周期、discard/fallback 和事件排序设计
- 迁移与回滚：先只开放 concurrency-safe 且参数已闭合的早执行路径；如效果或稳定性不佳，可关闭早执行，只保留候选组装与诊断数据，退回到现有完整输出后再执行的模式
