## Why

`astrcode-core` 的定位是"定义领域协议和跨 crate 共享的纯数据模型"，但当前 core 中混入了多处运行时逻辑和基础设施代码：

- `agent/input_queue.rs` 包含 `InputQueueProjection::replay_index()` 等回放算法（~115 行运行时逻辑）
- `runtime/coordinator.rs` 的 `RuntimeCoordinator` 包含 `RwLock`、mutable state、shutdown 编排（416 行有状态实现）
- `tool_result_persist.rs` 直接执行文件 I/O（470 行磁盘操作）
- `shell.rs` 通过 `Command::new` 执行进程检测（434 行系统调用）
- `project.rs` 包含 `fs::canonicalize` 等文件系统操作（219 行）
- `home.rs` 通过 `dirs::home_dir()` 读取宿主环境
- `plugin/manifest.rs` 在 core 中直接做 TOML 解析
- `error.rs` 让 `AstrError::HttpRequest` 直接绑定 `reqwest::Error`
- `agent/mod.rs` 挤了 ~60 个公开类型在单一文件中（1643 行）

core 当前还因此引入了 `dirs`、`reqwest`、`tokio`、`toml` 这些不该轻易出现在基础层的具体依赖。
其中 `reqwest`、`dirs`、`toml` 都直接对应 owner 错位的实现；`tokio` 则体现在能力/工具上下文里对 `UnboundedSender` 的直接绑定。

core 应该只定义类型和 trait，不持有运行时 owner，不做环境 I/O，也不绑死具体基础设施库。当前这些越界代码让 core 变重、变难测试、变难替换。

## What Changes

- 把 `InputQueueProjection` 的回放算法（`replay_index`、`replay_for_agent`、`apply_event_for_agent`）迁入 `session-runtime`，core 只保留 `InputQueueProjection` 与相关 envelope / payload DTO。
- 把 `RuntimeCoordinator` 从 core 迁到 `server` 组合根附近，core 只保留 `RuntimeHandle`、`ManagedRuntimeComponent` 等纯契约。
- 把 `tool_result_persist.rs` 拆成“共享协议 + 共享宿主实现”两层：core 保留 `PersistedToolResult`、`PersistedToolOutput`、路径/字符串解析 helper 与常量，文件落盘实现迁入 `astrcode-support::tool_results`。
- 把 `shell.rs` 拆成“共享 shell 类型 + 共享宿主实现”两层：core 保留 `ShellFamily`、`ResolvedShell` 等纯数据，shell 探测与命令存在性检查迁入 `astrcode-support::shell`。
- 把 `project.rs` 拆成“纯 project identity 算法 + 宿主路径解析”两层：core 保留 slug/hash 等纯字符串算法，`canonicalize`、home 目录解析与 project 路径拼装迁入 `astrcode-support::hostpaths`。
- 把 `home.rs` 迁出 core，由 `astrcode-support::hostpaths` 统一提供 home 目录解析，避免多个 crate 各自复制宿主路径逻辑。
- 把 `plugin/manifest.rs` 中的 TOML 解析迁出 core，core 只保留 `PluginManifest` 纯数据定义。
- 把 `AstrError::HttpRequest` 从 `reqwest::Error` 解耦为中立错误载体，移除 core 对具体 HTTP 客户端错误类型的绑定。
- 拆分 `agent/mod.rs` 为按职责组织的子模块，保留对外语义与 re-export 稳定。
- 明确 `TurnProjectionSnapshot` 本次暂不迁移：它仍是 `SessionRecoveryCheckpoint` / `EventStore` checkpoint 合同的一部分，待 checkpoint 边界后续拆分时再处理。
- 明确 `EventStore` trait 本次不拆分，除非 owner 迁移过程中出现真实的契约阻塞。

## Non-Goals

- 本次不引入泛化 `utils`/`helpers` 杂项桶；新增的 `astrcode-support` 只承载 `hostpaths`、`shell`、`tool_results` 这类边界明确的共享宿主能力。
- 本次不修改 core 中合理的类型定义和 trait 声明。
- 本次不修改 `kernel`（它只依赖 core，core 类型搬迁后 kernel 适配即可）。
- 本次不做 adapter 层的重组。
- 本次不迁移 `TurnProjectionSnapshot` 类型 owner，也不拆 `EventStore` checkpoint 合同。
- 本次不强行抽象 `tokio::sync::mpsc::UnboundedSender`；如果要把能力/工具上下文从具体 async runtime 解耦，单独开 change 处理更稳妥。

## Capabilities

### New Capabilities
- 无

### Modified Capabilities
- `core`: 职责严格收窄为"类型定义 + trait 声明 + port 定义"，不含运行时算法和基础设施代码。
- `session-runtime`: 接收 `InputQueueProjection` 回放算法，并继续作为会话 projection / projector 逻辑的唯一业务 owner。
- `application-use-cases`: 不再依赖 core-owned home / project helper 或 runtime owner，只消费稳定治理与路径契约。
- `adapter-contracts`: 接收 plugin manifest 解析等 adapter owner 变化，并改为消费 `astrcode-support` 提供的共享宿主能力。
- `astrcode-support`: 新增 `hostpaths`、`shell`、`tool_results` 模块，集中承接跨 crate 共享的宿主路径解析、shell 探测与工具结果持久化能力。

## Impact

- 影响面最大——core 被所有 crate 依赖，任何类型搬迁都会触发编译级联。
- 需要在 Change 2（session-runtime 边界稳定）之后执行，确保类型归属有明确的接收方。
- 仓库不追求向后兼容，优先以 core 的职责纯粹性为准。
- `server` 组合根会跟着调整，因为 `RuntimeCoordinator` 将不再由 core 导出，而改为在 bootstrap 附近拥有实现 owner。
