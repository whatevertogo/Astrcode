## Why

当前 `session-runtime` 在 LLM 输出被 `max_tokens` 截断时只记录 warning，没有把“从截断处继续”真正接回 turn loop。与此同时，`core` 配置里已经存在 `max_output_continuation_attempts`，说明仓库已经承认这个能力需要存在，只是还没有落到执行闭环里。

博客里最值得借鉴的一点就是：截断不是终点，而是下一轮输入的起点。Claude Code `query.ts` 里真正有价值的不是“再发一条继续”这么简单，而是把恢复尝试、最终失败释放时机和 loop transition 都做成显式状态。把这条恢复链补上，能显著提升长输出、多段推理和大上下文答复的连续性，而且不会破坏现有模块边界。

## What Changes

- 在 `session-runtime/turn` 中增加专门的输出截断恢复路径，把 `LlmFinishReason::MaxTokens` 从“只告警”升级为“可恢复继续”
- 为截断恢复引入稳定的 synthetic continuation prompt 语义，避免与 `CompactSummary`、`ReactivationPrompt` 混用
- 使用现有 `max_output_continuation_attempts` 作为硬上限，防止单次 turn 因连续截断而无限续写
- 在恢复仍可继续时暂缓把截断视为最终失败；只有达到上限或命中禁用条件后才给出最终停止结论
- 让 turn summary / observability 能稳定反映截断恢复次数、最终停止原因和放弃恢复的场景
- 不把恢复策略提升到 `application` 或 `kernel`；它继续属于 `session-runtime` 的 turn 真相
- 第一阶段不引入 provider 侧 `max_output_tokens` 升档或重配策略，先把 prompt-level continuation 闭环做好

## Capabilities

### New Capabilities

- 无

### Modified Capabilities

- `turn-orchestration`: turn loop 需要支持 `max_tokens` 截断后的继续恢复
- `turn-budget-governance`: 输出截断恢复需要受显式尝试上限与停止条件约束
- `turn-observability`: turn 汇总需要反映截断恢复的次数与退出原因

## Impact

- 受影响 crate：`crates/session-runtime`、`crates/core`
- 重点模块：`crates/session-runtime/src/turn/runner/step.rs`、`crates/session-runtime/src/turn/runner.rs`、`crates/session-runtime/src/turn/summary.rs`、`crates/core/src/action.rs` 或等价消息 origin 定义
- 用户可见影响：被模型输出上限截断的回答会在同一次 turn 内自动续写，而不是静默中断
- 开发者可见影响：`max_output_continuation_attempts` 将从纯配置项变成真正生效的执行约束，恢复中的中间截断不会被过早当作最终失败
- 迁移与回滚：先以内聚的 `continuation_cycle` 或等价模块接线；若恢复效果不稳定，可暂时保留消息 origin 与 summary 字段，但关闭自动继续逻辑
