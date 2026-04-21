## Why

Change 1 为 application 建立了稳定的 session orchestration contracts，但 `server` 仍然直接 `use astrcode_session_runtime` 的内部类型——特别是 `terminal_projection.rs` 直接匹配 10+ 个 `ConversationBlockFacts` 变体，`routes/conversation.rs` 直接实例化 `ConversationStreamProjector`，`routes/sessions/mutation.rs` 直接构造 `ForkPoint` 枚举。

这使得 session-runtime 的任何内部类型变更都会直接破坏 server 编译，application 的 anti-corruption layer 名义存在但 server 完全绕过了它。

## What Changes

- 在 application 层补全 terminal/conversation surface 的稳定合同（Change 1 只处理了 orchestration contracts，未覆盖 terminal read model）。
- 重写 `server/src/http/terminal_projection.rs`，改为消费 application 层的 terminal 合同类型而非直接 match session-runtime 的 Facts 枚举。
- 重写 `server/src/http/routes/conversation.rs`，通过 application 层的 stream 方法消费对话流，不再直接持有 `ConversationStreamProjector`。
- 重写 `server/src/http/routes/sessions/mutation.rs`，改为调用 application 层的 fork 用例，不再直接构造 `ForkPoint`。
- 移除 `server/Cargo.toml` 对 `astrcode-session-runtime` 的直接依赖。
- 移除 `server` 测试中对 `SessionState::append_and_broadcast` 的直接调用。
- 统一 `normalize_working_dir` 的调用路径，server 不再直接调用 session-runtime 的路径工具。

## Non-Goals

- 本次不重写 `astrcode-protocol` 的 HTTP DTO 结构。
- 本次不修改前端 SSE 事件格式。
- 本次不修改 session-runtime 内部结构（Change 1/2 的范围）。
- 本次不处理 `server` 的测试基础设施重构——只确保测试不再绕过 application 层。

## Capabilities

### New Capabilities
- 无

### Modified Capabilities
- `application-terminal-surface`: application 新增面向终端消费的 conversation snapshot / stream replay / fork 用例的稳定合同，server 作为消费者只通过这些合同与 session-runtime 交互。
- `server-http-routes`: HTTP 路由层不再直接 import session-runtime 类型，全部通过 application 用例方法消费。

## Impact

- 主要影响 `crates/server` 的 HTTP 层（terminal_projection、conversation routes、mutation routes）和 `crates/application` 的 terminal surface 导出面。
- `server/Cargo.toml` 删除 `astrcode-session-runtime` 依赖，可能需要在 application 层补充少量中间类型。
- server 测试需要改写为通过 application 层验证行为。
