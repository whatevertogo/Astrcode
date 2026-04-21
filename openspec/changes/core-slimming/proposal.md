## Why

`astrcode-core` 的定位是"定义领域协议和跨 crate 共享的纯数据模型"，但当前 core 中混入了多处运行时逻辑和基础设施代码：

- `agent/input_queue.rs` 包含 `InputQueueProjection::replay_index()` 等回放算法（~115 行运行时逻辑）
- `runtime/coordinator.rs` 的 `RuntimeCoordinator` 包含 `RwLock`、mutable state、shutdown 编排（416 行有状态实现）
- `tool_result_persist.rs` 直接执行文件 I/O（470 行磁盘操作）
- `shell.rs` 通过 `Command::new` 执行进程检测（434 行系统调用）
- `project.rs` 包含 `fs::canonicalize` 等文件系统操作（219 行）
- `TurnProjectionSnapshot` 仅被 session-runtime 消费，不应污染 core 的公共 API 面
- `agent/mod.rs` 挤了 ~60 个公开类型在单一文件中（1643 行）

core 应该只定义类型和 trait，不实现算法、不做 I/O、不持有可变状态。当前这些越界代码让 core 变重、变难测试、变难替换。

## What Changes

- 把 `InputQueueProjection` 的回放算法（`replay_index`、`replay_for_agent`、`apply_event_for_agent`）迁入 session-runtime，core 只保留数据结构定义。
- 把 `RuntimeCoordinator` 迁入 application 层（它本身就是应用基础设施）。
- 把 `tool_result_persist.rs` 的文件 I/O 逻辑迁入 adapter-storage 或独立模块，core 只保留 `PersistedToolResult` 等数据类型。
- 把 `shell.rs` 迁出 core（到 utility crate 或 application）。
- 把 `project.rs` 的文件系统操作迁出 core（到 utility crate 或 application）。
- 把 `TurnProjectionSnapshot` 迁入 session-runtime。
- 拆分 `agent/mod.rs` 为 `agent/types.rs`、`agent/collaboration.rs`、`agent/delivery.rs`、`agent/lineage.rs` 等子模块。
- 检查 `EventStore` trait 是否需要拆分为 `EventLogStore` + `SessionLifecycleStore`。

## Non-Goals

- 本次不引入新的 crate（如 utility crate），只做类型和逻辑的归属调整。如果 shell.rs/project.rs 需要新 crate，留到后续 change。
- 本次不修改 core 中合理的类型定义和 trait 声明。
- 本次不修改 `kernel`（它只依赖 core，core 类型搬迁后 kernel 适配即可）。
- 本次不做 adapter 层的重组。

## Capabilities

### New Capabilities
- 无

### Modified Capabilities
- `core`: 职责严格收窄为"类型定义 + trait 声明 + port 定义"，不含运行时算法和基础设施代码。
- `session-runtime`: 接收 `InputQueueProjection` 回放算法和 `TurnProjectionSnapshot`。
- `application`: 接收 `RuntimeCoordinator`。
- `adapter-storage`（或其他适配器）: 接收 `tool_result_persist` 的 I/O 逻辑。

## Impact

- 影响面最大——core 被所有 crate 依赖，任何类型搬迁都会触发编译级联。
- 需要在 Change 2（session-runtime 边界稳定）之后执行，确保类型归属有明确的接收方。
- 仓库不追求向后兼容，优先以 core 的职责纯粹性为准。
