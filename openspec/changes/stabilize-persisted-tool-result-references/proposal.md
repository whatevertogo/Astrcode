## Why

Astrcode 已经具备单个工具结果超长时的 `tool-results/` 落盘能力，但 `session-runtime` 仍然缺少 Claude Code 那种“按整条 tool-result message 做聚合预算、把大结果外置成稳定引用、恢复时重放相同 replacement 决策”的正式 loop 语义。现在补齐这条链路，才能让大工具输出既不压垮 prompt，又不退化成只剩占位文本的脆弱上下文。

## What Changes

- 把现有 `<persisted-output>` / `tool-results/` 机制从“单个工具的局部优化”提升为 turn/request 可理解的正式 prompt contract
- 在 `turn/request` 中增加 aggregate tool-result budget，对同一 API-level user tool-result 批次做统一预算裁剪，而不是只依赖单个工具自己的 inline limit
- 为 tool result replacement 引入稳定的 `tool_call_id -> replacement decision` 状态，并将其 durable 持久化，保证 resume / replay 后仍能重放完全一致的 replacement 文本
- 明确 `readFile("tool-results/...")` 是读取全量 persisted output 的标准回读路径，不为第一阶段再引入新的反馈摘要协议
- 为 persisted reference 命中率、重放次数、节省字节数和 over-budget message 数量增加 turn 级 observability
- 明确本次不把 UI/diagnostic summary 当成 prompt 主机制；工具反馈 summary 仅保留为第二阶段候选方向

## Capabilities

### New Capabilities

- 无

### Modified Capabilities

- `turn-orchestration`: turn/request 需要在组装 LLM 请求前对 tool-result batch 应用 persisted reference replacement
- `turn-budget-governance`: turn 内预算治理需要覆盖 message-level aggregate tool-result budget，而不是只看单个工具输出
- `turn-observability`: turn 汇总与诊断需要反映 persisted reference 命中、重放和节省量
- `session-persistence`: replacement decision 需要进入 durable event log，并在 session 恢复时重建

## Impact

- 受影响 crate：`crates/session-runtime`、`crates/core`、`crates/adapter-tools`
- 重点模块：`crates/session-runtime/src/turn/request.rs`、`crates/session-runtime/src/turn/runner/step.rs`、`crates/core/src/tool_result_persist.rs`、`crates/core/src/event/types.rs`、相关 projection / recovery 链路
- 用户可见影响：当 `grep`、`shell`、`findFiles` 等工具返回大量内容时，模型会优先看到稳定的 persisted reference，而不是随机截断或仅剩“旧结果已清除”的占位文本；需要全量内容时继续通过 `readFile("tool-results/...")` 读回
- 开发者可见影响：工具结果的“事实保存”与“prompt 消费”将被显式拆成两层，且 replacement 决策成为可恢复的 durable truth，而不是隐式内存状态
- 迁移与回滚：第一阶段仅引入 aggregate budget + durable replacement state；如效果不稳定，可关闭 request 侧 aggregate replacement，退回现有 per-tool persisted-output 与 prune/micro-compact 路径，同时保留 durable 事件与基础设施
