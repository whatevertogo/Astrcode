## Why

Change 1 完成后，application 的 port trait 和 contracts 已经整洁，但 application 内部有 5 个超过 1000 行的大文件，每个都承担了多种职责，难以沿单一主线理解：

- `agent/mod.rs`（1157 行）：`AgentOrchestrationService` 同时编排 spawn/send/observe/close 四工具的全部逻辑。
- `agent/terminal.rs`（1006 行）：混合了 child turn 终态收集、outcome 映射、parent delivery 构建与投递。
- `agent/wake.rs`（1182 行）：混合了父级 delivery 唤醒调度、reconcile、recovery 和 queued input 重排。
- `session_use_cases.rs`（1261 行）：`App` 上的 20+ 个 session 方法，涵盖 CRUD、submit、compact、observe、mode 等多个用域。
- `session_plan.rs`（1139 行）：plan workflow 状态管理与 `App` 的 impl 块紧耦合。

这些文件的共同问题不是"行数多"本身，而是**一个文件承载了多个可独立理解的用域**。当一个开发者需要理解"compact 用例怎么走"时，必须在 1261 行的 session_use_cases.rs 里找到 compact 相关的几个方法，中间隔着 submit、fork、mode 等完全不相关的逻辑。

## What Changes

- 拆分 `session_use_cases.rs` 按用域为独立文件：`session/crud.rs`、`session/submit.rs`、`session/compact.rs`、`session/observe.rs`、`session/mode.rs`。
- 拆分 `agent/mod.rs` 按工具为独立文件：`agent/orchestration.rs`、`agent/spawn.rs`、`agent/send.rs`、`agent/observe.rs`。
- 拆分 `agent/terminal.rs` 按关注点：`agent/terminal/outcome.rs`（turn 终态收集）、`agent/terminal/delivery.rs`（parent delivery 构建）。
- 拆分 `agent/wake.rs` 按关注点：`agent/wake/scheduler.rs`（唤醒调度主逻辑）、`agent/wake/reconcile.rs`（reconcile 与 recovery）。
- 把 `session_plan.rs` 的状态管理统一到 `workflow/` 子域，从 App 的 impl 中移出。

## Non-Goals

- 本次不修改 application 的 port trait 或公开 API——仅做内部文件组织。
- 本次不修改跨 crate 的依赖关系。
- 本次不新增子 crate。
- 本次不做性能优化或逻辑改动——纯文件移动和模块拆分。

## Capabilities

### New Capabilities
- 无

### Modified Capabilities
- `application-internal-structure`: 文件组织从"大文件多职责"变为"一文件一用域"，公开 API 不变。

## Impact

- 纯内部重组，不影响 `application` 的公开 API 表面或 port trait 签名。
- 不影响 `server`、`session-runtime` 或其他 crate 的编译。
- 测试代码可能需要调整 import 路径，但逻辑不变。
