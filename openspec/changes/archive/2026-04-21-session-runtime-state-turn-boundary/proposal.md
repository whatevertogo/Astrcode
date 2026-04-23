## Why

`linearize-session-runtime-application-boundaries`（Change 1）已经解开了 `session-runtime` 内部的大部分重复与反向依赖，但明确延后了最关键的一步：把 turn 运行时控制状态从 `state/` 彻底移出，让 `state`、`turn`、`query` 三条线真正各归其位。

当前真实代码里仍然存在三个边界问题：

- `crates/session-runtime/src/state/mod.rs` 同时持有投影注册表、writer、广播器，以及 `TurnRuntimeState` / `CompactRuntimeState` / `ActiveTurnState` / `ForcedTurnCompletion` 等运行时控制状态。
- `crates/session-runtime/src/query/service.rs` 仍然承载 `wait_for_turn_terminal_snapshot()` 这种带订阅等待循环的运行时协调逻辑，导致 `query` 既做纯读，又做 watcher。
- `turn` 子域虽然已经拥有 `TurnCoordinator`，但控制状态的 prepare / complete / interrupt / cancel 仍然要经由 `SessionState` 代理，开发者无法沿着单一主线理解“谁拥有 turn runtime truth”。

需要说明的是，proposal 初稿里提到的 `turn/replay.rs` 归位问题已经在前一轮整理中完成：`session_replay()` / `session_transcript_snapshot()` 现在已经位于 `query/replay.rs`。本次 change 不重复制造过期任务，而是聚焦还没有收口的 state / turn / watcher 边界。

另一个必须写清楚的前提是：`server-session-runtime-isolation` 负责先把 `server` / `application` 测试从 `SessionState` runtime proxy 上摘下来。本 change 会删除这些 proxy，因此它不能先于 isolation 独立落地；若两者叠在同一实现栈中，也必须先完成 isolation 的测试收口，再删除 proxy。

## What Changes

- 把 `TurnRuntimeState`、`CompactRuntimeState`、`ActiveTurnState`、`ForcedTurnCompletion`、`PendingManualCompactRequest` 从 `state/mod.rs` 迁入 `turn/runtime.rs`，让这些类型由 `turn` 子域定义和维护。
- 让 `SessionActor`（或等价的单 session live truth owner）直接持有 `TurnRuntimeState`；`SessionState` 收窄为 durable projection state + writer + broadcaster 的 owner，不再持有或代理 turn runtime control。
- 把 `wait_for_turn_terminal_snapshot()` 从 `query/service.rs` 迁入 `turn/watcher.rs`（或等价的 turn-owned watcher 模块），让 `query` 保持“纯读快照 / 回放”，不再持有订阅等待循环。
- 调整 `SessionRuntime`、`turn/submit`、`turn/interrupt`、`query/service`、`actor` 等消费方，改由 turn runtime handle 或 watcher 读取 / 推进控制状态，同时保持 `SessionRuntime` crate 根门面稳定。
- 同步更新 `PROJECT_ARCHITECTURE.md` 与 `session-runtime` 模块注释，明确三层分离：
  - durable event + projection truth
  - turn runtime control state
  - external pure-data snapshots

## Non-Goals

- 本次不修改 turn terminal projector、compact 事件序列、conversation projection 等已在 Change 1 收口的读模型逻辑。
- 本次不修改 `application` / `server` 的合同与跨 crate ACL。
- 本次不修改 `core` / `kernel` 的结构。
- 本次不引入新的 hooks、workflow 或 mode contract 抽象。
- 本次不改变 `SessionRuntime` 根门面对外的公开 API 语义，只调整内部 owner 和模块归属。

## Capabilities

### New Capabilities

- 无

### Modified Capabilities

- `session-runtime`: `SessionState` 的职责收窄为 durable projection state、writer 与广播基础设施；turn runtime control state 迁入 `turn` 子域并由单 session live truth owner 持有。
- `session-runtime-subdomain-boundaries`: `turn` 子域完整拥有 turn runtime control 与 watcher；`query` 子域只保留纯读与回放，不再承载等待循环。

## Impact

- 主要影响 `crates/session-runtime` 内部的 `state/`、`turn/`、`query/`、`actor/`、`lib.rs`。
- 需要新增 `turn/runtime.rs` 与 `turn/watcher.rs`（或等价文件），并重写部分 `state/mod.rs`、`query/service.rs`、`turn/submit.rs`、`turn/interrupt.rs`、`turn/finalize.rs`、`command/mod.rs` 的内部调用链。
- 会影响 `session-runtime` 相关单测与模块注释，也会影响 `application` / `server` 中直接操纵 `SessionState` 运行时 proxy 的测试辅助代码；这些测试需要改走稳定的 runtime 测试路径，但不改变正式运行时合同。
- 与 `server-session-runtime-isolation` 存在显式实施顺序依赖：应先完成 HTTP/test 边界收口，再删除 `SessionState` runtime proxy；否则外部 crate 测试会在中间状态下失去编译路径。
