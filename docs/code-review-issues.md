# Code Review Issues — 2026-03-31

> 基于 Phase 1 (assembly 下沉) 和 Phase 2 (Capability 统一) 完成后的全量审查。

---

## 🔴 High — 需要立即修复

### H1. `llm/anthropic.rs` 与 `llm/openai.rs` 大量重复代码

6 处几乎相同的代码：`is_retryable_status`、`build_http_client`、`wait_retry_delay`、`emit_event`、
常量 `CONNECT_TIMEOUT/READ_TIMEOUT/MAX_RETRIES/RETRY_BASE_DELAY_MS`、测试 helper `sink_collector`。

**修复**: 抽取到 `llm/mod.rs` 共享模块。

### H2. `CapabilityDescriptor` 等类型在 `core` 和 `protocol` 中重复定义

`CapabilityDescriptor`、`CapabilityKind`、`PermissionHint`、`SideEffectLevel`、`StabilityLevel`、
`CapabilityDescriptorBuilder` 在两个 crate 里各自独立定义。
`crates/plugin/src/invoker.rs` 有 ~65 行机械转换函数。

**修复**: 统一到 protocol 定义 wire 格式、core 引用 protocol 类型，消除重复。

### H3. `phase_of_storage_event` 逻辑重复 + Bug

- `event/translate.rs:40-50` 和 `event/query.rs:349-358` 有两份相同的 phase 映射函数。
- **Bug**: 两者都将 `StorageEvent::Error` 映射到 `Phase::Idle`，
  但 translate 的实际逻辑里 "interrupted" 错误会设为 `Phase::Interrupted`，
  导致被中断的会话在 metadata 中报告错误的 phase。

**修复**: 保留 `translate.rs` 的 `phase_of_storage_event`，`query.rs` 调用它；修复 Error 分支逻辑。

### H4. `ToolResult` 存储时丢失 error/metadata 信息

`StorageEvent::ToolResult` 没有 `error`/`metadata` 字段，translate 后永远为 `None`。
工具执行的错误详情在 replay 时无法恢复。

**修复**: 在 `StorageEvent::ToolResult` 中添加 `error: Option<String>` 和 `metadata: Option<Value>` 字段。

---

## 🟡 Medium — 值得处理

### M1. `duration_ms` 类型不一致

| 位置 | 类型 |
|------|------|
| `action.rs:37` (`ToolExecutionResult`) | `u128` |
| `registry/capability.rs:56` (`CapabilityExecutionResult`) | `u128` |
| `event/types.rs:55` (`StorageEvent::ToolResult`) | `u64` |

**修复**: 统一为 `u64`（584 百万年精度足够）。

### M2. `AgentContext` 从未使用

`sdk/src/agent.rs` 定义了 `AgentContext` 但未被 re-export 或引用。

**修复**: 删除文件和模块声明。

### M3. `MemoryProvider` trait 无实现

`sdk/src/memory.rs` 声明了 trait 但无任何实现。

**修复**: 如无近期计划，删除或标记 `#[doc(hidden)]`。

### M4. `Transport` trait 和 `WebSocketTransport` 无实际实现

- `protocol/src/transport/traits.rs` 的 `Transport` trait 没有实现者（stdio 在 plugin crate）。
- `plugin/src/transport/websocket.rs` 全是 stub 错误。

**修复**: 删除 `WebSocketTransport` stub；将 `Transport` trait 移至 plugin crate 或统一。

### M5. `HandlerDispatcher` 可能已被 `CapabilityRouter` 取代

`plugin/src/handler_dispatcher.rs` 只有 register 和 descriptors，无 dispatch 逻辑。

**修复**: 确认后删除。

### M6. `ToolCapabilityInvoker` 硬编码 descriptor 元数据

`core/registry/tool.rs:124-137` 将所有 tool 标记为 `builtin`、`Workspace` side-effect、`Stable`。

**修复**: 让 `Tool` trait 提供 descriptor 或 metadata 回调。

### M7. `std::sync::Mutex` 用于 async context

`server/src/auth.rs:48` 使用 `std::sync::Mutex<HashMap>`。

**修复**: 替换为 `tokio::sync::Mutex` 或 `DashMap`。

### M8. `peer.rs` spawn_read_loop 丢弃 JoinHandle

`plugin/src/peer.rs:125-130` spawn 后丢弃 handle，无法强制关闭。

**修复**: 存储 `JoinHandle`，shutdown 时 abort。

### M9. service 层双错误策略

`runtime/service/support.rs` 同时提供 `ServiceResult` 和 `anyhow::Result` 的工具函数。

**修复**: 统一到 `ServiceError`。

### M10. `StreamWriter::records()` 用 `.expect()` 可能 panic

`sdk/src/stream.rs:86-87`。

**修复**: 改为返回 `Result`。

### M11. `eprintln!` 而非 `log::warn!` / `log::debug!`

`runtime/agent_loop/turn_runner.rs:137` 和 `runtime/agent_loop/llm_cycle.rs:46`。

**修复**: 替换为 `log::warn!` / `log::debug!`。

---

## 🟢 Low — 代码卫生

### L1. `registry/capability.rs` 是纯转发文件

`core/registry/capability.rs` 只有一行 `pub use crate::capability::{...}`。

**修复**: 合并到 `registry/mod.rs` 或直接删除。

### L2. `dashmap` 依赖声明但未使用

`core/Cargo.toml` 声明了 `dashmap.workspace = true` 但无代码使用。

**修复**: 移除依赖。

### L3. `CapabilityKind::custom()` 与 `new()` 完全相同

`core/capability.rs:36-38`。

**修复**: 删除 `custom()`。

### L4. `declare_tool!` macro 未被使用

`sdk/src/macros.rs`。

**修复**: 删除或标记 `#[doc(hidden)]`。

### L5. `From<String/&str>` 映射到 `SdkError::Validation` 语义不正确

`sdk/src/error.rs:201-217`。

**修复**: 添加 `SdkError::Internal` 变体来接收任意错误字符串。

### L6. 公共 enum 缺 `#[non_exhaustive]`

`SdkError`、`ToolSerdeStage`、`HookShortCircuit`、`EventPhase`、`PluginMessage` 等。

**修复**: 添加 `#[non_exhaustive]`。

### L7. `#[allow(unused_imports)]` 可能掩盖未使用的 re-export

`runtime/prompt/mod.rs:11,16`。

**修复**: 确认外部 crate 确实使用，否则删除。

### L8. `SessionState.working_dir` 已存但从未被读取

`runtime/service/session_state.rs:94`。

**修复**: 确认用途后删除或使用。

### L9. host peer descriptor 仍用 `"astrcode-server"`

`runtime/runtime_surface_assembler.rs:358`。

**修复**: 改为 `"astrcode-runtime"` 或提取为常量。

### L10. 中文硬编码错误信息（无 i18n）

runtime 和 server 多处。

**修复**: 记录，暂不处理（需 i18n 基础设施）。

---

## 修复状态追踪

| # | 状态 | 说明 |
|---|------|------|
| H1 | ✅ | 抽取 LLM 共享代码到 `llm/mod.rs` |
| H2 | ⬜ | 统一 core/protocol 类型（架构决策，暂缓） |
| H3 | ✅ | 修复 phase 映射 Bug + 重复（Interrupted 分支修复 + 函数统一） |
| H4 | ⬜ | StorageEvent 添加 error/metadata（影响存储格式，暂缓） |
| M1 | ✅ | duration_ms → u64（action.rs + router.rs + translate.rs） |
| M2 | ✅ | 删除 AgentContext |
| M3 | ✅ | 删除 MemoryProvider |
| M4 | ✅ | 删除 WebSocketTransport stub |
| M5 | ✅ | 删除 HandlerDispatcher |
| M6 | ⬜ | Tool descriptor 元数据硬编码（需设计 Tool trait 扩展） |
| M7 | ⬜ | auth.rs Mutex（需评估改为 tokio::sync::Mutex 的影响） |
| M8 | ⬜ | peer.rs JoinHandle（需评估生命周期影响） |
| M9 | ⬜ | service 错误策略统一 |
| M10 | ✅ | StreamWriter records() 返回 Result |
| M11 | ✅ | eprintln → log::warn/log::debug |
| L1 | ✅ | 删除 registry/capability.rs 转发文件 |
| L2 | ✅ | 移除 dashmap 依赖 |
| L3 | ✅ | CapabilityKind::custom() 保留并加文档注释 |
| L4 | ✅ | declare_tool! 标记 #[doc(hidden)] |
| L5 | ⬜ | From<String> → SdkError 语义 |
| L6 | ⬜ | 公共 enum 加 #[non_exhaustive] |
| L7 | ✅ | #[allow(unused_imports)] 清理 |
| L8 | ⬜ | SessionState.working_dir dead code |
| L9 | ✅ | host peer descriptor → "astrcode-runtime" |
| L10 | ⬜ | 中文硬编码（需 i18n 基础设施） |
