## Why

`linearize-session-runtime-application-boundaries` 已经把 `application` 的 session orchestration contracts 拉直，但 `server` 仍然在 HTTP 层直接 `use astrcode_session_runtime` 的内部类型。最明显的例子是：

- `crates/server/src/http/terminal_projection.rs` 直接匹配 10+ 个 `ConversationBlockFacts` / `ConversationDeltaFacts` 变体
- `crates/server/src/http/routes/conversation.rs` 直接实例化 `ConversationStreamProjector`
- `crates/server/src/http/routes/sessions/mutation.rs` 直接构造 `ForkPoint`
- `crates/server/src/http/routes/sessions/mod.rs` 直接调用 `normalize_working_dir`
- `crates/server/src/tests/*` 直接穿透 `_runtime_handles.session_runtime.get_session_state()` 操作 `SessionState`

这使得 `session-runtime` 的任何内部类型演化都会直接破坏 `server` 编译，`application` 作为 anti-corruption layer 只在部分路径上存在，terminal / conversation / fork / route test 这条链路仍然被 `server` 直接绕过。

更具体地说，当前问题不只是 `server` 自己 import 了 runtime。`AppSessionPort` 仍然直接暴露 `ConversationSnapshotFacts`、`ConversationStreamReplayFacts`、`ForkPoint`、`ForkResult`、`SessionControlStateSnapshot`、`SessionReplay` 等一整组 runtime 类型，而 `crates/application/src/terminal/mod.rs` 的 `TerminalFacts.transcript` / `TerminalStreamReplayFacts.replay` 仍然把 runtime transcript / replay 继续向上透传。这意味着只要 terminal mapper 或 conversation route 继续消费这两个字段，`server` 就无法真正从 runtime read model 退回到 `application` 语言。

需要明确的是，`App::fork_session()` 目前已经把 runtime `ForkResult` 收口成 `SessionMeta` 返回给 `server`。因此本次 change 的目标不是“证明所有 runtime 类型都已经泄漏到 `server`”，而是把这些残留泄漏点正式收口成 `application` 自己拥有的边界合同。

## What Changes

- 在 `application` 层补全 terminal / conversation surface 的稳定合同：`TerminalFacts.transcript`、`TerminalStreamReplayFacts.replay` 不再直接承载 runtime snapshot/replay；`application` 自己拥有 block / delta / patch / status / snapshot / replay / rehydrate / authoritative summary 等完整合同面。
- 重写 `crates/server/src/http/terminal_projection.rs`，改为消费 `application` 的 terminal surface contracts，而不是直接匹配 runtime Facts。
- 重写 `crates/server/src/http/routes/conversation.rs`，通过 `application` 的 terminal stream surface 获取 replay / delta / rehydrate 结果；stream projection 的协调状态迁入 `application`，server 不再直接持有 runtime projector。
- 重写 `crates/server/src/http/routes/sessions/mutation.rs`，改为消费 `application` 自己的 fork selector 合同，不再直接构造 runtime `ForkPoint`；同时把“runtime `ForkResult` 只留在 application/port 内部、server 只拿 `SessionMeta`”写成正式边界。
- 重写 `crates/server/src/http/routes/sessions/mod.rs` 的工作目录校验路径，server 不再直接调用 runtime 的 `normalize_working_dir`。
- 重写 `crates/server/src/tests/*` 与 `crates/server/src/http/routes/conversation.rs` 内 route-local tests 中直接操作 runtime internals 的测试，改为通过 `application` surface 或语义化 test harness 驱动场景。
- 把 `server` 对 `astrcode-session-runtime` 的直接使用收缩到 bootstrap / 明确的内部 test harness；保留 crate 级依赖，因为 `server` 仍然是组合根。
- 在 change 文档中显式声明与 `session-runtime-state-turn-boundary` 的实施顺序：先完成本 change 的 HTTP/test 收口，再删除 `SessionState` runtime proxy。

## Non-Goals

- 本次不重写 `astrcode-protocol` 的 HTTP DTO 结构。
- 本次不修改前端 SSE 事件格式。
- 本次不修改 `session-runtime` 内部 read model 或 stream projector 的算法语义。
- 本次不移除 `server` crate 对 `astrcode-session-runtime` 的 crate 级依赖；bootstrap 组合根仍可直接引用 runtime。
- 本次不做全面的 `server` 测试基础设施翻修，只处理 route contract tests 对 runtime internals 的直接穿透。

## Capabilities

### New Capabilities
- `server-http-routes`: 约束 HTTP route、route mapper 与 route contract tests 只通过 `application` 稳定合同消费业务能力。

### Modified Capabilities
- `application-use-cases`: application 扩展 terminal / conversation surface 的稳定合同与 stream projection 协调能力，使 `server` 只消费 application-owned session facts。
- `session-fork`: fork 用例在 `server -> application` 边界上改为使用 application-owned selector，而不是 runtime `ForkPoint`。

## Impact

- 主要影响 `crates/application/src/terminal*`、`crates/application/src/session_use_cases.rs`、`crates/application/src/ports/app_session.rs`，以及 `crates/server/src/http/*` 和 `crates/server/src/tests/*`。
- `server` 的 crate 依赖不变，但 route / mapper / tests 的 import 面会显著收口。
- terminal/conversation contract 面比初稿估算更大：需要覆盖 8 种 block、3 种 delta、10 种 patch、4 种 status，以及 plan / system note / child handoff / transcript error 等叶子结构与枚举，避免 `server` 再承接 runtime 内部枚举与投影器。
