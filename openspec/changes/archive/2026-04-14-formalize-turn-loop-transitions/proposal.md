## Why

当前 `session-runtime` 的 turn loop 已经完成了 `assemble -> LLM -> tool -> continue/end` 的结构化拆分，但“为什么进入下一轮”仍然分散在 `StepOutcome`、局部计数器和临时分支里。这样虽然能工作，却让已经写进 spec 的 budget-driven auto-continue、后续的截断恢复和流式工具调度都缺少稳定的状态骨架。

Claude Code `query.ts` 值得借鉴的不是“把所有逻辑塞回一个巨型循环”，而是它把 `transition.reason` 做成了一等概念。现在补齐这层显式 transition 建模，是为了把 turn loop 从“可以继续跑”提升到“可以稳定扩展、稳定观测、稳定测试”的状态，同时不破坏 `session-runtime -> kernel -> application` 的既有边界。

## What Changes

- 在 `session-runtime/turn` 中引入显式的 loop transition / stop cause 语义，统一表达 step 级 continue 与 turn 级 stop 的原因
- 让 budget-driven auto-continue 正式接到 turn loop 的显式 transition 模型，而不是继续停留在分散的局部判断里
- 让 turn summary 与相关事件能够稳定暴露“最后一次 transition 原因”和“最终 stop 原因”，为后续截断恢复与流式调度打基础
- 保持 `application` 只负责发起 turn，不承接 loop 内继续/停止策略
- 不在 `kernel` 中引入新的 loop façade，也不把 loop 状态泄漏到协议层 DTO
- 明确这套语义是对现有模块化 runner 的加强，而不是回退到单个 `queryLoop()` 风格实现

## Capabilities

### New Capabilities

- 无

### Modified Capabilities

- `turn-orchestration`: turn loop 需要显式表达 continue transition，而不是仅靠隐式分支推进
- `turn-budget-governance`: budget 决策需要输出稳定的 continuation/stop cause，并接到正式 loop 语义
- `turn-observability`: turn 汇总需要暴露 transition / stop cause，避免治理层猜测 loop 行为

## Impact

- 受影响 crate：`crates/session-runtime`、`crates/core`
- 重点模块：`crates/session-runtime/src/turn/runner.rs`、`crates/session-runtime/src/turn/runner/step.rs`、`crates/session-runtime/src/turn/summary.rs`、`crates/session-runtime/src/turn/events.rs`
- 用户可见影响：单次 turn 的停止原因和续写原因会更稳定，可用于后续 UI/诊断展示
- 开发者可见影响：后续新增 `max_tokens` 恢复、流式工具调度时不再需要继续堆叠隐式分支
- 迁移与回滚：先在 `session-runtime` 内部补充 transition/stop cause 类型并接线；如果实现效果不理想，可以保留新类型但暂时回退到现有 loop 分支，不影响现有 API 入口
