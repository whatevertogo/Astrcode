## Context

`PROJECT_ARCHITECTURE.md` 已经明确了两条边界：

- `server` 是组合根，但业务交互必须通过 `application`
- `server` 的 HTTP 路由不应直接 import `session-runtime` 的内部类型

当前实现和这两条边界仍有明显偏差：

- `crates/server/src/http/terminal_projection.rs` 直接依赖 `astrcode_session_runtime::ConversationBlockFacts`、`ConversationDeltaFacts`、`ToolCallBlockFacts` 等内部 read-model 类型
- `crates/server/src/http/routes/conversation.rs` 直接实例化 `ConversationStreamProjector`
- `crates/server/src/http/routes/sessions/mutation.rs` 直接构造 runtime `ForkPoint`
- `crates/server/src/http/routes/sessions/mod.rs` 直接调用 runtime `normalize_working_dir`
- `crates/server/src/tests/session_contract_tests.rs` 与 `config_routes_tests.rs` 直接通过 `_runtime_handles.session_runtime.get_session_state()` 操作 `SessionState`

与此同时，`application` 虽然已经拥有 `TerminalFacts` / `TerminalStreamFacts` / `terminal_queries`，但这层 surface 还不够彻底：

- `crates/application/src/terminal/mod.rs` 中 `TerminalFacts.transcript` 仍然直接承载 runtime `ConversationSnapshotFacts`
- `TerminalStreamReplayFacts.replay` 仍然直接承载 runtime `ConversationStreamReplayFacts`
- `crates/application/src/ports/app_session.rs` 仍然直接 import 了 `ConversationSnapshotFacts`、`ConversationStreamReplayFacts`、`ForkPoint`、`ForkResult`、`SessionCatalogEvent`、`SessionControlStateSnapshot`、`SessionModeSnapshot`、`SessionReplay`、`SessionRuntime`、`SessionTranscriptSnapshot` 等一整组 runtime 类型
- `App::fork_session()` 和 `AppSessionPort::fork_session()` 仍然把 runtime `ForkPoint` 暴露到 `server -> application` 边界；虽然 `App::fork_session()` 已经把 `ForkResult` 收口成 `SessionMeta` 返回给 `server`，但这个边界还没有在 change 文档里被明确固定

所以这次 change 的核心不是“把 `server` 和 runtime 完全断开”，而是：

1. 让 `application` 真正拥有 terminal / conversation / fork 的稳定合同
2. 让 `server` 退回到 HTTP 解析、鉴权、状态码映射、SSE framing 与 DTO 映射
3. 把 `server` 对 runtime 的直接使用收口到组合根和必要的内部测试夹具

## Goals / Non-Goals

**Goals:**

- 为 terminal / conversation surface 定义 application-owned contracts，不再把 runtime `Conversation*Facts` 继续暴露给 `server`
- 让 `server` 的 conversation snapshot / stream / fork / session create 路由只消费 `application` surface
- 把 runtime `ConversationStreamProjector` 的使用移出 `server` HTTP route，改由 `application` 拥有相应的 stream projection 语义
- 把 fork 输入从 runtime `ForkPoint` 改成 application-owned selector
- 把 server route contract tests 对 `SessionState` 的直接穿透改为语义化 test harness

**Non-Goals:**

- 不修改前端 HTTP/SSE DTO 结构
- 不改变 conversation read model 的 block / delta 业务语义
- 不重写 `session-runtime` 内部 projector / reducer 实现
- 不移除 `server` crate 对 `astrcode-session-runtime` 的 crate 级依赖；bootstrap 仍然允许直接引用 runtime
- 不把所有 server 测试一次性改成完全黑盒，只先修 route contract tests 的 runtime internals 穿透

## Cross-Change Dependency

`session-runtime-state-turn-boundary` 会删除 `SessionState::prepare_execution()`、`is_running()` 等 runtime proxy，并已经识别到 `application` / `server` 测试对这些方法存在直接依赖。因此两个 change 的建议实施顺序必须写清楚：

1. 先完成 `server-session-runtime-isolation`，把 HTTP route 与 route tests 收口到 `application` surface / 语义化 harness。
2. 再实施 `session-runtime-state-turn-boundary`，删除 `SessionState` 的 runtime proxy。

如果两个 change 需要叠在同一实现栈内提交，也必须先落完本 change 的测试收口，再删除 proxy；否则 `server` / `application` 测试会在中间状态下失去可编译路径。

## Decisions

### Decision 1: 保留 `server` 作为组合根对 runtime 的 crate 级依赖，但禁止该依赖继续进入 HTTP 路由层

本次 change 不会删除 `server/Cargo.toml` 中的 `astrcode-session-runtime` 依赖。

原因：

- `server` 仍然是组合根，bootstrap 必须直接组装 `application`、`kernel`、`session-runtime` 与 adapters
- `PROJECT_ARCHITECTURE.md` 允许 server 在 bootstrap 中直接引用核心 runtime 层
- 真正的问题不是 crate 级依赖，而是 route / mapper / tests 把 runtime 内部类型当成了业务合同

因此本次采用更硬的边界：

- 允许：`crates/server/src/bootstrap/**`、明确受限的 test harness 内部使用 runtime
- 禁止：`crates/server/src/http/**`、route mapper、route contract tests 直接使用 runtime 的 read-model / helper / enum

替代方案是强行删除 `server` 对 runtime 的 crate 级依赖。这会与组合根职责冲突，也会把 bootstrap 组装逻辑强行绕进 `application`，因此不采用。

### Decision 2: `application` 成为 terminal / conversation surface 的合同 owner

`application` 不再只返回“带 runtime facts 的终端摘要”，而是要真正拥有自己的 terminal contracts。推荐把 `crates/application/src/terminal/` 拆成更清晰的子模块：

- `terminal/contracts.rs`: 定义 application-owned block / delta / patch / status / snapshot / replay / rehydrate / control / child / slash contracts
- `terminal/runtime_mapping.rs`: 把 runtime `Conversation*Facts` 映射为 application contracts
- `terminal/stream_projection.rs`: 持有 stream replay / authoritative summary 的 projection 协调
- `terminal/summary.rs`: 保留当前 summary helper

关键约束：

- `TerminalFacts.transcript` 与 `TerminalStreamReplayFacts.replay` 必须替换成 application-owned snapshot / replay 类型
- server 不再看到 runtime `ConversationBlockFacts`、`ConversationDeltaFacts`、`ConversationBlockPatchFacts`、`ConversationStreamReplayFacts`
- application 返回的 terminal contracts 必须是纯数据结构
- server 仍然保留协议 DTO 映射，不把 `astrcode-protocol` 引入 `application`

这层 contract 不能只停留在顶层 rename，因为 `crates/server/src/http/terminal_projection.rs` 当前逐个匹配了：

- 8 种 `ConversationBlockFacts` 变体
- 3 种 `ConversationDeltaFacts` 变体
- 10 种 `ConversationBlockPatchFacts` 变体
- 4 种 `ConversationBlockStatus`
- plan / tool call / system note / child handoff / transcript error 等叶子结构与枚举

因此本次 design 明确要求 `application` 在 terminal / conversation 边界拥有完整的语义合同面，而不是只用 runtime 叶子类型做薄包装。`SessionControlStateSnapshot` 可以继续作为 `application` 内部查询输入参与映射，但不得再作为 terminal surface 的返回字段回流到 `server`。

这样做的原因：

- terminal / conversation surface 是一个正式业务入口，应该由 `application` 拥有稳定合同
- server 只应做 transport 适配，而不是接过 runtime 的内部 read model 直接继续解释
- 这能把 “runtime 事实 -> app 事实 -> protocol DTO” 这条链路拉直

替代方案是保留 application 当前的 `TerminalFacts` 结构，只在 server 做一层“少 import 一点”的薄包装。这不会消除 server 对 runtime read model 的耦合，因此不采用。

### Decision 3: stream projection 协调迁到 `application`，server 只保留 SSE 循环与 framing

当前 `routes/conversation.rs` 里有两层不该留在 server 的语义：

- authoritative summary 的维护
- `ConversationStreamProjectorState` 对 replay / agent event 的投影协调

本次改为由 `application` 提供稳定的 stream projection surface。这里显式选择“状态搬迁而不是算法搬迁”：

1. `application` 暴露 app-owned `ConversationStreamProjectionState`（或等价的 projection 协调器）
2. 该状态内部允许继续持有 runtime `ConversationStreamProjector`
3. `SessionEventRecord` / `AgentEvent` 的投影协调逻辑搬到 `application/terminal/stream_projection.rs`
4. server 只通过 `application` 暴露的 `seed_initial_replay` / `push_durable_record` / `push_live_event` / `recover_from` / `apply_authoritative_refresh` 等等价接口消费结果

不采用“投影算法整体搬迁到 application”的方案，因为这会直接违反本 change 的 Non-Goals。

server 在这一设计下只负责：

- 鉴权
- query/path 解析
- 调 `app.conversation_stream_*`
- 把 application delta 映射成 protocol DTO
- 把 DTO 包成 SSE envelope

替代方案是把 SSE route 整体搬进 `application`。这会把 transport concern 混回业务层，不采用。

### Decision 4: fork 输入选择器改成 application-owned contract

当前 `server -> application` 仍然通过 runtime `ForkPoint` 交互，这与 Change 1 的 anti-corruption 目标不一致。与此同时，`App::fork_session()` 已经把 runtime `ForkResult` 收口成 `SessionMeta` 返回给 `server`，所以本次要做的是把这条实际边界正式固定下来，而不是让 `server` 再次观察 `ForkResult` 字段。

本次引入 application-owned selector，例如：

```rust
pub enum SessionForkSelector {
    Latest,
    TurnEnd { turn_id: String },
    StorageSeq { storage_seq: u64 },
}
```

边界重新划分为：

- server 解析 HTTP body -> `SessionForkSelector`
- `application::App::fork_session(session_id, selector)` 处理输入校验与用例编排
- `AppSessionPort` blanket impl 在 port 内部把 selector 映射为 runtime `ForkPoint`
- runtime `ForkResult` 只允许停留在 port / use case 内部，`App::fork_session()` 对外继续只返回 `SessionMeta`

这样做的原因：

- fork 点解析属于应用合同，不应由 server 继续知道 runtime enum
- 这和 `session_id`、terminal facts 的治理方向一致：`application` 对上游暴露自己的语言

替代方案是继续让 `server` 构造 runtime `ForkPoint`，只把其余 terminal surface 拉回 `application`。这会保留一个明显的 runtime leak，因此不采用。

### Decision 5: working-dir 规范化回到 application 用例入口

`routes/sessions/mod.rs` 当前直接调用 runtime `normalize_working_dir()`。这不符合 “server 只做 transport，use case 校验归 application” 的边界。

本次改为：

- server 只做空值/JSON 形状校验
- `App::create_session` / 对应 use case 负责 working-dir 校验与规范化失败的业务错误映射
- runtime 仍然保留 canonical helper，但调用点下沉到 application / port 内部

这样做的原因：

- working-dir 是否有效是业务输入校验，不是 route 应自行理解的 runtime 规则
- server 不再需要直接 import runtime path helper

替代方案是把 `normalize_working_dir` 复制到 server。那只会制造第二个 owner，因此不采用。

### Decision 6: route contract tests 改为语义化 harness，不再直接穿透 `SessionState`

当前 server contract tests 通过 `_runtime_handles.session_runtime.get_session_state()` 直接操作：

- `writer.append()`
- `translate_store_and_cache()`
- `broadcaster.send()`
- `prepare_execution()`

这使 route tests 和 runtime 内部状态机绑死。

本次改为在 `crates/server/src/test_support.rs` 或等价位置增加语义化 helper，例如：

- `seed_completed_root_turn(...)`
- `seed_unfinished_root_turn(...)`
- `mark_session_running(...)`

这些 helper 可以在内部暂时继续使用 runtime handles，但 test body 不再直接碰 `SessionState`。

这样做的原因：

- 先切断“测试直接理解 runtime internals”的耦合
- 不把整个 server 测试基础设施重写并入本 change

替代方案是要求所有 route tests 都改成全黑盒 HTTP 驱动，不再有任何内部夹具。这方向长期成立，但会明显放大变更面，因此不采用。

## Files

**重点新增文件：**

- `crates/application/src/terminal/contracts.rs`
  - 定义 application-owned terminal / conversation contracts，覆盖 block / delta / patch / status / replay / rehydrate / 相关叶子结构
- `crates/application/src/terminal/runtime_mapping.rs`
  - 承接 runtime facts -> application contracts 的映射
- `crates/application/src/terminal/stream_projection.rs`
  - 承接 `ConversationStreamProjectorState` 的 projection 协调；内部允许继续使用 runtime projector

**重点修改文件：**

- `crates/application/src/terminal/mod.rs`
  - 从“混合 runtime facts + summary helper”调整为模块入口
- `crates/application/src/terminal_queries/snapshot.rs`
  - 改为返回 application-owned terminal surface
- `crates/application/src/session_use_cases.rs`
  - fork / create session 用例切换到 application-owned selector 与输入校验，并保持 `App::fork_session()` 只返回 `SessionMeta`
- `crates/application/src/ports/app_session.rs`
  - blanket impl 内部完成 fork selector -> runtime `ForkPoint` 的映射，并收口 runtime `ForkResult`
- `crates/application/src/lib.rs`
  - 收口 terminal surface 的公开导出
- `crates/server/src/http/terminal_projection.rs`
  - 改为只映射 application contracts -> protocol DTO
- `crates/server/src/http/routes/conversation.rs`
  - 改为消费 application-owned stream surface
- `crates/server/src/http/routes/conversation.rs`（tests）
  - route-local tests 改为构造 application-owned stream facts，而不是 runtime replay facts
- `crates/server/src/http/routes/sessions/mutation.rs`
  - 改为消费 application-owned fork selector
- `crates/server/src/http/routes/sessions/mod.rs`
  - 移除 runtime working-dir helper 直连
- `crates/server/src/tests/session_contract_tests.rs`
- `crates/server/src/tests/config_routes_tests.rs`
  - route contract tests 改为语义化 harness

## Risks / Trade-offs

- [风险] application terminal contracts 可能过度贴近 protocol DTO，形成第二套 transport 模型。  
  → 缓解：contract 只表达业务语义，不直接复用 protocol DTO 命名或 HTTP 细节。

- [风险] terminal / conversation contract 面比初稿更大，若低估工作量，迁移中容易留下半条 runtime 泄漏路径。  
  → 缓解：把 block / delta / patch / status / 叶子枚举的完整清单写入 design 与 tasks，按 contract inventory 逐项收口。

- [风险] stream projector 迁移到 application 后，delta 序列可能与现有 route 行为不一致。  
  → 缓解：只搬迁协调状态，不重写 runtime projector 算法；增加 snapshot / replay / catch-up 等价测试。

- [风险] fork selector 迁移会同时改动 `App`、`AppSessionPort`、server route 与 test support。  
  → 缓解：先引入 selector，再在同一 change 内删除 runtime `ForkPoint` 在 server/application 边界上的暴露。

- [风险] route tests 不再直接碰 `SessionState` 后，某些极端场景更难搭建。  
  → 缓解：允许 test harness 内部暂时保留 runtime handles，但禁止在测试主体直接操作。

- [风险] 只收口 route 层，bootstrap alias 仍然存在，团队可能误以为 server 任意模块都可继续 import runtime。  
  → 缓解：在 spec 和任务里显式限定 runtime 使用只允许留在 bootstrap / internal harness。

## Migration Plan

1. 先在两个 change 中声明实施顺序：`server-session-runtime-isolation` 先于 `session-runtime-state-turn-boundary`。
2. 在 `application` 引入完整的 terminal contracts inventory（block / delta / patch / status / snapshot / replay / rehydrate）并替换 `TerminalFacts.transcript` / `TerminalStreamReplayFacts.replay` 的 runtime 字段。
3. 在 `application` 引入 runtime facts -> app contracts 的映射与 stream projection 协调层，明确只搬迁协调状态，不重写 runtime projector 算法。
4. 修改 `application` terminal queries / session use cases / app session port，使其不再向上暴露 runtime terminal facts 和 runtime fork enum，并固定 `App::fork_session()` 只返回 `SessionMeta`。
5. 重写 `server` 的 terminal projection mapper 与 conversation / session routes。
6. 重写 route contract tests 与 conversation route-local tests，改为语义化 harness / application-owned fixtures。
7. 清理 `server` HTTP 层残留的 runtime imports，并执行边界检查。

回滚策略：

- 若 stream projection 迁移导致 SSE 行为异常，可先保留新的 application contracts，但短期恢复 server 侧旧投影实现；这不会影响持久化数据。
- 若 fork selector 迁移影响面过大，可在一次提交内保留 application 内部的 runtime `ForkPoint` 兼容映射，但不让该类型继续回流到 server。

## Open Questions

- terminal contracts 是否应一步拆成多个子文件，还是先保留在 `terminal/mod.rs` 下再逐步拆分？
- server test harness 是否需要独立模块（如 `tests/harness.rs`），还是先放入现有 `test_support.rs`？
- `bootstrap/deps.rs` 中的 `session_runtime` alias 是否需要额外文档化为“组合根专用依赖”，避免后续再次扩散？
