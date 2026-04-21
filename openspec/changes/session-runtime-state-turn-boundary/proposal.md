## Why

`linearize-session-runtime-application-boundaries`（Change 1）解开了 session-runtime 内部的重复与反向依赖，但明确延后了 `TurnRuntimeState` / `CompactRuntimeState` 从 `state/` 到 `turn/` 的搬家，以及 `replay.rs` 的归位和 `wait_for_turn_terminal_snapshot` 的迁移。

Change 1 完成后，`state/mod.rs` 仍然同时持有投影注册表（事件溯源世界）和 turn 运行时状态机（运行时世界）。`turn/replay.rs` 仍然是只读查询但放在执行模块中。`query/service.rs` 仍然承载异步等待循环。这使得 state/ 和 turn/ 的边界仍然模糊，开发者无法沿单一主线理解"投影在哪、运行时状态在哪、等待逻辑在哪"。

## What Changes

- 把 `TurnRuntimeState`（含嵌套的 `CompactRuntimeState`、`ActiveTurnState`、`ForcedTurnCompletion`）从 `state/mod.rs` 整体迁入 `turn/runtime.rs`，使 `SessionState` 只持有投影注册表 + writer + broadcaster。
- 把 `turn/replay.rs` 的 `SessionRuntime` 扩展方法迁入 `query/replay.rs`（或 `query/transcript.rs`），使只读查询全部归入 query 子域。
- 把 `query/service.rs` 中的 `wait_for_turn_terminal_snapshot` 异步等待逻辑迁入独立的 `turn/watcher.rs`（或等价模块），使 query 层保持纯读投影语义。
- 调整 `SessionState` 的方法代理层：原来转发到 `TurnRuntimeState` 的方法（`prepare_execution`、`complete_execution_state`、`cancel_active_turn`、`interrupt_execution_if_running` 等）改为由 `turn/` 模块直接持有和操作 turn runtime state。
- 同步更新 `actor/`、`command/`、`turn/submit`、`turn/interrupt` 等消费方，让它们从 turn runtime state 的新的归属位置获取控制能力。

## Non-Goals

- 本次不修改投影逻辑或 compact 事件序列（已在 Change 1 完成）。
- 本次不修改 application 或 server 的合同（已在 Change 1 和将在 Change 3 完成）。
- 本次不调整 `kernel` 或 `core` 的结构。
- 本次不拆分 `ProjectionRegistry` 的子 reducer（已在 Change 1 完成）。

## Capabilities

### New Capabilities
- 无

### Modified Capabilities
- `session-runtime-state`: `SessionState` 职责收窄为"投影注册表 + 存储写入 + 事件广播"，不再持有运行时控制状态。
- `session-runtime-turn`: turn 子域完整拥有自身的运行时控制状态机（prepare/complete/interrupt/cancel）和 turn 终态等待能力。
- `session-runtime-query`: query 子域完整拥有所有只读查询能力，包括历史回放。

## Impact

- 主要影响 `crates/session-runtime` 内部的 `state/`、`turn/`、`query/`、`actor/` 子模块。
- `SessionState` 的公开方法签名可能调整（部分方法从 SessionState impl 移到 turn runtime），但不改变外部 crate 的调用方式——`SessionRuntime` 根门面保持稳定。
- 需要更新大量内部测试的调用路径。
