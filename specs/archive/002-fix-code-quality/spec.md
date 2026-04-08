# Feature Specification: 修复项目代码质量问题

**Feature Branch**: `002-fix-code-quality`
**Created**: 2026-04-08
**Status**: Draft
**Input**: User description: "修复 CODE_QUALITY_ISSUES.md 中记录的所有项目质量问题"

## User Scenarios & Testing

### User Story 1 - 修复编译阻断问题 (Priority: P1)

作为开发者，我需要项目能够通过 `cargo check --workspace` 和 `cargo clippy` 检查，这样我才能正常开发和提交代码。

**Why this priority**: 编译错误阻断所有开发工作，必须最先修复。

**Independent Test**: 运行 `cargo check --workspace` 和 `cargo clippy --all-targets --all-features -- -D warnings` 全部通过。

**Acceptance Scenarios**:

1. **Given** `watch_ops.rs:332` 存在 `&Cow<str>` 的 Pattern trait bound 编译错误，**When** 修复后，**Then** `cargo check --workspace` 通过无错误。
2. **Given** `filter.rs:3` 有 unused import 警告，**When** 移除后，**Then** `cargo clippy` 不再报告该警告。

### User Story 2 - 解除 core 对 protocol 的违规依赖 (Priority: P1)

作为架构维护者，我需要 `core` 和 `protocol` 之间不存在任何直接依赖（宪法 1.2.0 Architecture Constraints），这样分层架构才能保持清晰。

**Why this priority**: 这是宪法明确要求的双向独立约束，且当前 core 依赖 protocol 是最高优先级的架构违规。

**Independent Test**: 检查 `crates/core/Cargo.toml` 的 `[dependencies]` 不包含 `astrcode-protocol`；`cargo check --workspace` 通过。

**Acceptance Scenarios**:

1. **Given** `core/Cargo.toml` 依赖了 `astrcode-protocol`，**When** 将 protocol 中的 Plugin 相关类型移入 core 后，**Then** `core` 不再依赖 `protocol` 且所有功能正常。
2. **Given** `core/src/capability.rs` 从 protocol re-export 了 Plugin 相关类型，**When** 这些类型直接定义在 core 中后，**Then** re-export 路径消除，编译通过。

### User Story 3 - 消除生产代码中的 panic 路径 (Priority: P1)

作为运行时维护者，我需要所有锁获取、数组索引、超时等待不使用 `.unwrap()` / `.expect()`，这样生产环境不会因 panic 崩溃。

**Why this priority**: 宪法 VI Runtime Robustness 明确禁止 panic 路径。当前至少有 13 处生产代码存在 panic 风险。

**Independent Test**: 搜索非测试代码中的 `.unwrap()` 和 `.expect()` 调用，在锁获取、数组索引、channel 操作中不再存在。

**Acceptance Scenarios**:

1. **Given** `plugin/src/peer.rs:300` 使用 `.lock().unwrap()`，**When** 替换为恢复机制后，**Then** 锁中毒时返回错误而非 panic。
2. **Given** `runtime-registry/src/router.rs` 有 9 处 `.expect()`，**When** 替换为安全锁获取后，**Then** 锁获取失败返回 `Result` 而非 panic。
3. **Given** `runtime-agent-control/src/lib.rs:608` 使用 `.expect("waiter should finish before timeout")`，**When** 替换为 `match` 处理后，**Then** timeout 超时返回错误而非 panic。

### User Story 4 - 修复异步任务泄漏和持锁 await (Priority: P1)

作为并发安全维护者，我需要所有 `tokio::spawn` 创建的任务都有句柄管理和取消机制，且不存在持锁 await 模式。

**Why this priority**: 宪法 VI 明确要求。当前有 4 处 fire-and-forget 和 1 处持锁 await，可能导致资源泄漏和死锁。

**Independent Test**: 搜索 `tokio::spawn` 调用，每个都有 `JoinHandle` 被保存和管理；搜索 `.lock().await.*.await` 模式不存在。

**Acceptance Scenarios**:

1. **Given** `plugin/src/peer.rs:293` spawn 后无 handle，**When** 保存 handle 并提供取消机制后，**Then** 任务生命周期可管理。
2. **Given** `plugin/src/supervisor.rs:200` 在持锁状态下 await shutdown，**When** 重构为释放锁后再 await 后，**Then** 不再阻塞其他任务。
3. **Given** `runtime/src/service/watch_manager.rs` 和 `execution/subagent.rs`、`execution/root.rs` 的 fire-and-forget spawn，**When** 保存 handle 后，**Then** 所有异步任务都有生命周期管理。

### User Story 5 - 统一错误处理链路 (Priority: P2)

作为代码质量维护者，我需要各 crate 的错误类型与 `core::AstrError` 兼容，且错误转换不丢失上下文。

**Why this priority**: 错误处理不一致会导致上层调用者难以统一处理，但不阻断编译。

**Independent Test**: 各 crate 的自定义错误类型实现 `Into<AstrError>` 或通过 `#[source]` 保留原始错误；`map_err(|_| ...)` 丢弃上下文的用法不再存在。

**Acceptance Scenarios**:

1. **Given** 6 个 crate 各自定义错误类型，**When** 统一后，**Then** 上层只需处理 `AstrError` 一种错误类型。
2. **Given** `src-tauri/src/main.rs:424` 使用 `map_err(|_| ...)` 丢弃原始错误，**When** 修复后，**Then** 原始错误信息被保留。

### User Story 6 - 修正日志级别和消除静默错误 (Priority: P2)

作为运维人员，我需要关键操作有正确的日志级别，且错误不被静默吞掉。

**Why this priority**: 宪法 VII Observability 要求。影响问题诊断效率。

**Independent Test**: 搜索 `runtime-agent-loop/src/hook_runtime.rs` 和 `core/src/runtime/coordinator.rs` 中的关键错误不再使用 `debug!`；搜索 `.ok()` 和 `let _ =` 在非测试代码中的使用都有注释说明。

**Acceptance Scenarios**:

1. **Given** turn failed 用 `warn!` 而非 `error!`，hook call failed 用 `debug!` 而非 `error!`，**When** 修正后，**Then** 所有关键操作失败使用 `error!`。
2. **Given** `storage/src/session/event_log.rs:254` 文件操作失败被 `.ok()` 忽略，**When** 修复后，**Then** 错误被记录或返回。
3. **Given** `runtime-config/src/loader.rs:92` 使用 `println!`，**When** 替换为 `log::warn!` 后，**Then** 生产代码无 `println!/eprintln!` 残留。

### User Story 7 - 拆分过大的 service 模块（两阶段） (Priority: P2)

作为代码维护者，我需要 `runtime/src/service/` 下单文件不超过 800 行（宪法 1.2.0 约束）。此拆分在 US1-US4 完成并稳定后执行。

**Why this priority**: 宪法 Architecture Constraints 的量化约束。需要先稳定边界重构再拆分，避免同时做两件大事。

**Independent Test**: `wc -l` 统计 service 目录下所有 `.rs` 文件，均不超过 800 行。

**Acceptance Scenarios**:

1. **Given** `service/mod.rs` 约 17,000 行，**When** 按职责拆分为独立子模块后，**Then** 每个文件不超过 800 行，功能不变。
2. **Given** `execution/mod.rs` 约 14,000 行，**When** 拆分后，**Then** root execution、subagent、status 各为独立文件且不超过 800 行。

### User Story 8 - 消除硬编码常量并统一依赖版本 (Priority: P3)

作为项目维护者，我需要所有硬编码的端口号、大小限制等值提取为常量，且所有依赖版本统一到 workspace。

**Why this priority**: 不影响功能但影响维护性。

**Independent Test**: 搜索代码中的裸数字常量（62000、128000、20000、200000）不再存在；各 Cargo.toml 中的 toml、tracing、async-stream、tower 依赖使用 `workspace = true`。

**Acceptance Scenarios**:

1. **Given** 端口号 62000 在两个文件中硬编码，**When** 提取为 `core/src/env.rs` 中的常量后，**Then** 两处引用同一常量。
2. **Given** 4 个依赖未使用 workspace，**When** 统一后，**Then** 所有依赖版本由根 Cargo.toml 集中管理。

### Edge Cases

- Plugin 类型从 protocol 移入 core 时，protocol 中任何引用这些类型的传输 DTO 需要通过 mapper 重新映射。
- service/mod.rs 拆分时，需要确保拆分后的模块间调用关系不产生新的循环依赖。
- 锁获取替换为恢复机制时，需要确认 `with_lock_recovery` 在所有场景下都能正确恢复。

## Requirements

### Functional Requirements

- **FR-001**: 项目 MUST 通过 `cargo check --workspace` 无编译错误
- **FR-002**: 项目 MUST 通过 `cargo clippy --all-targets --all-features -- -D warnings` 无警告
- **FR-003**: `core` crate MUST NOT 依赖 `protocol` crate（宪法 Architecture Constraints）
- **FR-004**: Plugin 相关类型（PluginDescriptor、PluginInterface 等）MUST 从 protocol 移入 core 定义
- **FR-005**: 生产代码中锁获取 MUST NOT 使用 `.unwrap()` 或 `.expect()`，MUST 使用恢复机制或返回 `Result`
- **FR-006**: 生产代码中数组索引 MUST 使用安全变体（`.get()`、`.get_mut()`）或前置长度断言
- **FR-007**: 所有 `tokio::spawn` 创建的任务 MUST 保存 `JoinHandle` 并具备取消机制
- **FR-008**: 代码中 MUST NOT 存在持锁 await 模式（`lock().await` 后再 `.await`）
- **FR-009**: 各 crate 的错误类型 MUST 与 `core::AstrError` 兼容（实现 `Into<AstrError>` 或通过 source chain）
- **FR-010**: `map_err` 转换 MUST 保留原始错误信息，MUST NOT 使用 `map_err(|_| ...)` 丢弃上下文
- **FR-011**: 关键操作失败 MUST 使用 `error!` 日志级别
- **FR-012**: 生产代码 MUST NOT 使用 `println!` 或 `eprintln!`
- **FR-013**: `.ok()` 和 `let _ =` MUST 仅在注释说明原因时使用
- **FR-014**: `runtime/src/service/` 下单文件 MUST NOT 超过 800 行
- **FR-015**: 硬编码的端口号、大小限制等值 MUST 提取为 `core/src/env.rs` 或 `runtime-config/src/constants.rs` 中的常量
- **FR-016**: 所有 Cargo.toml 依赖版本 MUST 统一使用 workspace 继承
- **FR-017**: 所有改动后 MUST 通过 `cargo test --workspace --exclude astrcode` 验证

### Key Entities

- **AstrError**: core 中定义的主错误类型，所有 crate 的错误最终必须能转换为此类型
- **with_lock_recovery / lock_anyhow**: runtime-session 中已有的安全锁获取工具，需推广到其他 crate
- **JoinHandle 管理器**: 需要设计统一的异步任务生命周期管理机制

## Success Criteria

### Measurable Outcomes

- **SC-001**: `cargo check --workspace && cargo clippy --all-targets --all-features -- -D warnings` 零错误零警告
- **SC-002**: `core/Cargo.toml` 中不包含 `astrcode-protocol` 依赖
- **SC-003**: 生产代码中锁获取的 `.unwrap()` / `.expect()` 数量为 0
- **SC-004**: 生产代码中 fire-and-forget `tokio::spawn` 数量为 0
- **SC-005**: `runtime/src/service/` 下所有文件不超过 800 行
- **SC-006**: 所有 Cargo.toml 依赖使用 `workspace = true` 或有正当理由直接指定
- **SC-007**: `cargo test --workspace --exclude astrcode` 全部通过

## Clarifications

### Session 2026-04-08

- Q: core→protocol 依赖解耦时，Plugin 相关类型（PluginDescriptor、PluginInterface 等）应归属到哪一层？ → A: 将 Plugin 类型从 protocol 移入 core。Plugin 是核心扩展机制，属于"稳定契约与共享领域类型"，protocol 只是传输 DTO 层。
- Q: 锁获取失败时（StdMutex 中毒或 tokio Mutex 失败）应采用什么恢复策略？ → A: 保持现有 Mutex 类型，统一使用 `poisoned.into_inner()` 恢复 + `log::error!` 记录，与现有 `with_lock_recovery` 模式一致。tokio Mutex 同样用 `into_inner()` 恢复。
- Q: service 模块拆分应采用什么粒度策略？ → A: 两阶段拆分：先完成当前已进行的边界重构（US1-US4），稳定后再拆分 service 模块。避免同时做边界重构和模块拆分两件大事。
- Q: 各 crate 的错误类型应如何与 `AstrError` 统一？ → A: 在 `AstrError` 中新增对应 variant + `#[source]` 保留原始错误（如 `AstrError::Protocol { #[source] inner: ProtocolError }`），各 crate 保持自有错误类型。
- Q: 异步任务的取消和生命周期管理应采用什么模式？ → A: 各模块自行保存 `JoinHandle` 到结构体字段，通过已有的 `CancelToken` 或 `CancellationToken` 触发 abort，无全局管理器。与现有 CancelToken 模式一致。

## Assumptions

- `with_lock_recovery` 机制可以在所有 crate 中使用，或每个 crate 可以实现类似的安全机制
- service 模块拆分不会改变外部 API 行为，只是内部代码组织
- 拆分过程中可能需要临时放宽 800 行限制，但最终所有文件必须在限制内
