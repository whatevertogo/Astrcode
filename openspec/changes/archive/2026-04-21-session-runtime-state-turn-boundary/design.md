## Context

`session-runtime` 在上一轮整理后，已经把大量重复投影、反向依赖和 `application` 泄漏收口到了更清晰的结构里，但 `state`、`turn`、`query` 三个子域仍然没有完全形成单向主线。

当前代码事实如下：

- `state/mod.rs` 既持有 `ProjectionRegistry`、`SessionWriter`、`broadcast::Sender`，又内嵌 `TurnRuntimeState`、`CompactRuntimeState`、`ActiveTurnState`、`ForcedTurnCompletion`。
- `query/service.rs` 的 `wait_for_turn_terminal_snapshot()` 通过订阅 `state.broadcaster` 做循环等待。这是运行时 watcher，不是纯读查询。
- `turn/submit.rs` 已经有 per-turn 的 `TurnCoordinator`，但 prepare / complete / interrupt 等控制动作仍经由 `SessionState` 代理，导致 `turn` 的运行时控制能力实际散落在 `state` 内部。
- `query/replay.rs` 已经拥有 `session_replay()` 和 `session_transcript_snapshot()`，说明“只读回放归 `query`”这一点已经实现，本次无需重复搬家。

这使得开发者仍然需要同时打开 `state/mod.rs`、`turn/submit.rs`、`query/service.rs` 才能拼出“turn 当前在哪跑、谁负责终止、谁在等待终态、谁只负责读快照”。问题不在 Rust，而在 owner 没有被写清楚。

本次 change 的目标不是再做一轮大重构，而是把剩余最关键的边界补全：

- `SessionState` 只做 durable / projected truth
- `turn` 子域定义并驱动 runtime control truth
- `query` 子域只做 pure-data query / replay

## Cross-Change Dependency

`server-session-runtime-isolation` 与本 change 之间存在显式顺序依赖：

1. `server-session-runtime-isolation` 先把 `server` / `application` 的 route tests 与本地 test support 从 `SessionState` runtime proxy 上摘下来。
2. 本 change 再删除 `SessionState::prepare_execution()`、`complete_execution_state()`、`is_running()` 等 proxy。

如果两者需要叠在同一代码栈内推进，也必须先完成 isolation 的测试迁移，再删除 proxy。否则 `application` / `server` 将在中间状态下直接编译失败。

## Goals / Non-Goals

**Goals:**

- 明确 `TurnRuntimeState` 的模块 owner 与 live owner，消除 `SessionState` 对运行时控制状态的直接拥有。
- 让 `query` 子域只保留纯读和回放语义，去掉等待循环。
- 保持 `SessionRuntime` 根门面和外部调用语义稳定，不把内部重构扩散到 `application` / `server`。
- 让 `state -> turn -> query` 的职责边界可以从目录和代码结构上直接读出来。
- 更新架构文档与模块注释，使本次边界调整成为正式约束而不是口头共识。

**Non-Goals:**

- 不修改 turn projector、conversation projector、compact event builder 等上一轮已经稳定的投影算法。
- 不引入新的跨 crate DTO 或重新设计 `application` / `server` 合同。
- 不把 `wait_for_turn_terminal_snapshot()` 进一步演化成通用 observe framework；本次只做 owner 归位。
- 不调整 `ProjectionRegistry` reducer 结构，也不继续拆分新的投影域。
- 不改变 `SessionRuntime` 对外公开方法的功能语义。

## Decisions

### Decision 1: `turn/runtime.rs` 定义运行时控制类型，`SessionActor` 直接持有 `TurnRuntimeState`

本次不会把 `TurnRuntimeState` 继续塞回 `SessionState`，也不会把它提升到 `SessionRuntime` 这种全局目录级 owner。采用的结构是：

- `turn/runtime.rs` 定义并维护 `TurnRuntimeState`、`CompactRuntimeState`、`ActiveTurnState`、`ForcedTurnCompletion`、`PendingManualCompactRequest`
- `SessionActor` 直接持有 `turn_runtime: TurnRuntimeState` 作为单 session live control truth
- `SessionState` 不再持有 `TurnRuntimeState`

选择这个方案的原因：

- `SessionActor` 已经是单 session live truth owner，生命周期与单个 session 对齐，天然适合托管“只在进程内存在的运行时控制状态”。
- 如果继续让 `SessionState` 持有，只是“文件搬家”，owner 仍然混在 durable projection owner 里，边界问题不会消失。
- 如果把 runtime control 提升到 `SessionRuntime` 全局 map，会破坏单 session 局部性，还会把全局目录变成第二个状态中心。
- `SessionActor` 自身已经通过 `Arc<SessionActor>` 共享，`TurnRuntimeState` 内部也已经使用 `Atomic*` + `StdMutex` 维护并发；在 actor 里再包一层 `Arc<TurnRuntimeState>` 没有明确收益，只会让 owner 再次变糊。

备选方案：

- 方案 A：只把类型移到 `turn/runtime.rs`，但 `SessionState` 继续持有。拒绝原因：owner 未变，问题本质未解。
- 方案 B：让 `SessionRuntime` 的 `LoadedSession` 直接持有 runtime control，actor 不知道它。拒绝原因：会让单 session live truth 被拆成两半，`actor` 与 `turn` 的协作会更绕。

### Decision 2: `SessionState` 收窄为 durable projection state + 存储/广播基础设施

`SessionState` 在本次重构后只保留以下职责：

- `ProjectionRegistry`
- `SessionWriter`
- durable / live broadcaster
- durable/projection 相关 getter
- event append / translate / cache 等围绕 durable truth 的方法

迁移后的 `SessionState` 保留清单如下：

- `projection_registry: StdMutex<ProjectionRegistry>`
- `broadcaster: broadcast::Sender<SessionEventRecord>`
- `live_broadcaster: broadcast::Sender<AgentEvent>`
- `writer: Arc<SessionWriter>`
- `translate_store_and_cache()`
- `append_and_broadcast()`
- `recent_records_after()`
- `snapshot_recent_stored_events()`
- `snapshot_projected_state()`
- `current_phase()`
- `current_mode_id()`
- `last_mode_changed_at()`
- `current_turn_messages()`
- `turn_projection()`
- `recovery_checkpoint()`
- `subscribe_live()` / `broadcast_live_event()`

它不再承担以下职责：

- `prepare_execution()`
- `complete_execution_state()`
- `force_complete_execution_state()`
- `interrupt_execution_if_running()`
- `cancel_active_turn()`
- `compacting()` / `set_compacting()` / `request_manual_compact()`
- `active_turn_id_snapshot()` / `manual_compact_pending()` / `is_running()` 这类 runtime control snapshot
- `PendingManualCompactRequest` 的定义与所有权

这些能力全部改由 `TurnRuntimeState` 或围绕它的 turn-owned helper 暴露。

这样做的收益：

- `SessionState` 的数据面会重新变成“可恢复真相 + 事件广播”的单一线条。
- `SessionState` 的测试也会从“投影 + runtime control 混测”回到 durable/projection 语义。

### Decision 3: `wait_for_turn_terminal_snapshot()` 迁入 `turn/watcher.rs`

`wait_for_turn_terminal_snapshot()` 不是纯 query：

- 它订阅 broadcaster
- 它在 lagged / closed 时做恢复性回放
- 它本质上是在等待 turn runtime 走到可判定终态

因此它应归到 `turn` 子域，由新的 `turn/watcher.rs`（或等价模块）拥有。推荐结构：

- `turn/watcher.rs` 提供 `SessionTurnWatcher<'a>` 或等价 helper
- `turn/watcher.rs` 一并拥有 `try_turn_terminal_snapshot()`、`try_turn_terminal_snapshot_from_recent()`、`turn_snapshot_is_terminal()`、`record_targets_turn()`、`turn_events()`
- `SessionRuntime::wait_for_turn_terminal_snapshot()` 直接委托给 turn watcher
- `query/service.rs` 去掉等待循环，只保留 one-shot snapshot / stored event / conversation snapshot / control snapshot 读取
- `split_records_at_cursor()` 继续留在 `query/service.rs`，因为它只服务 conversation stream replay，不属于 turn watcher

备选方案：

- 继续放在 `query/service.rs`。拒绝原因：`query` 无法保持“拉取即返回”的纯读语义。
- 放入 `observe/`。拒绝原因：当前需求不是统一订阅框架，只是 turn 终态等待；把它塞进 `observe` 会引入额外概念。

### Decision 4: replay 保持在 `query`，本次只写成显式不变量，不重复制造迁移任务

proposal 初稿里提到“把 `turn/replay.rs` 迁入 `query`”，但真实代码中这件事已经完成：

- `session_replay()` 位于 `query/replay.rs`
- `session_transcript_snapshot()` 位于 `query/replay.rs`

因此本次设计不再把 replay 搬家作为实现任务，而是把它固化成显式边界：

- `query` 拥有 replay / transcript / snapshot 读取
- `turn` 不再拥有任何 replay/read-only helper

这能避免 change 文档和真实代码继续漂移。

### Decision 5: `SessionRuntime` 根门面保持稳定，内部调用链改为 actor-owned runtime handle

对外不新增新的 facade 层，也不要求上层理解 `TurnRuntimeState`。根门面仍保留：

- `SessionRuntime::wait_for_turn_terminal_snapshot()`
- `SessionRuntime::list_running_sessions()`
- `SessionRuntime::session_control_state()`

但内部实现改为：

- 先从已加载 session 拿到 `SessionActor`
- 再从 actor 读取 `TurnRuntimeState`
- 使用 turn-owned runtime / watcher 读取或推进运行时控制状态

这样能同时满足两个目标：

- 外层调用不变
- 内层 owner 清晰

需要额外处理的一点是：`application` 和 `server` 当前各自有测试直接调用 `SessionState::prepare_execution()`、`complete_execution_state()`、`is_running()`。这些不是正式合同，但删除 proxy 后会立刻编译失败。

本次不通过给 `session-runtime` 增加 `#[cfg(test)]` 跨 crate helper 来解决，因为依赖 crate 的 `cfg(test)` 项不会自动暴露给外部 crate 的测试。正确做法是：

- 把这些测试迁移到各自 crate 内的稳定测试路径
- 优先复用 `SessionRuntime` 根门面和既有行为入口
- 必要时在调用方 crate 自己的 test support 中封装 helper，而不是继续把 `SessionState` 暴露为跨 crate 运行时控制入口
- 实施顺序上先落 `server-session-runtime-isolation` 的测试收口，再删除 proxy；否则外部 crate 没有稳定替代入口

### Decision 6: recovery 仍然把 runtime control state 初始化为空闲

`TurnRuntimeState` 从 `state` 移走后，崩溃恢复语义保持不变：

- durable display phase 继续由 checkpoint + tail events 恢复
- runtime control state 一律以 idle 初始化

这意味着：

- `SessionActor::from_recovery()` 在构建 `SessionState` 后，同时构建一个空闲的 `TurnRuntimeState`
- 恢复后的 running / active turn 都必须为 idle/none

这样不会引入“崩溃前的 active turn 还能继续跑”的假象。

### Decision 7: 文档与模块注释同步成为约束的一部分

本次不是纯内部实现整理。`PROJECT_ARCHITECTURE.md` 与相关模块注释必须同步：

- `state`：durable projection state + storage/broadcast infra
- `turn`：runtime control + execution + watcher
- `query`：pure read / replay / snapshot
- 外部扩展点仍然只拿纯数据，不暴露 runtime primitive

否则新边界很容易在后续迭代里再次漂移。

## Risks / Trade-offs

- [Risk] `SessionActor` 同时持有 `SessionState` 和 `TurnRuntimeState`，会让“单 session live truth owner”变宽。  
  → Mitigation：在实现中把两者明确区分为 durable/projection truth 与 runtime control truth，并只通过窄 getter 暴露 runtime handle。

- [Risk] `SessionRuntime::list_running_sessions()`、`session_control_state()` 等调用链需要一起改，容易漏掉 runtime snapshot 读取点。  
  → Mitigation：任务里单列 runtime snapshot caller 清理，并用 `rg` + `cargo check -p astrcode-session-runtime -p astrcode-application -p astrcode-server` 验证。

- [Risk] 把 watcher 从 `query` 移出后，相关测试可能大量失效。  
  → Mitigation：迁移现有 `query/service.rs` 中的 watcher 单测到 `turn/watcher.rs`，并保留 `SessionRuntime` facade 级回归测试。

- [Risk] 如果只是“文件搬家”，`SessionState` 仍通过 proxy 方法间接拥有 runtime control，边界不会真正改善。  
  → Mitigation：明确把 runtime proxy 从 `SessionState` 删除，而不是保留兼容壳。

- [Risk] `query` 去掉等待循环后，调用方可能误以为 `query` 还能做运行时协调。  
  → Mitigation：在 spec 与模块注释里把“query 只做 pure read”写成正式约束。

- [Risk] `application` / `server` 的测试当前直接操纵 `SessionState` runtime proxy，删除 proxy 后会让边界修复被测试代码阻塞。  
  → Mitigation：把这些测试列为显式迁移任务，改走调用方本地 test support 或稳定 runtime façade，而不是回退保留 `SessionState` proxy。

## Migration Plan

1. 先新增 `turn/runtime.rs` 和 turn-owned runtime tests，把运行时控制类型搬过去。
2. 让 `SessionActor` 直接持有 `TurnRuntimeState`，同时删除 `SessionState` 的 runtime 字段和 proxy 方法。
3. 改 `submit` / `interrupt` / `finalize` / `command` / `list_running_sessions` / `session_control_state` 等路径，统一经 actor 的 runtime handle 访问控制状态。
4. 新增 `turn/watcher.rs`，迁移 `wait_for_turn_terminal_snapshot()` 及其专属 helper 与对应测试。
5. 在 `server-session-runtime-isolation` 已经收口测试边界后，迁移 `application` / `server` 中直接依赖 `SessionState` runtime proxy 的测试辅助代码；若两者叠栈，必须先落该 change 的对应测试迁移，再继续本步。
6. 清理 `query/service.rs` 的 watcher 逻辑、过期注释与无效 helper。
7. 更新 `PROJECT_ARCHITECTURE.md` 和 `session-runtime` 模块注释。

回滚策略：

- 若 watcher 或 runtime owner 迁移中出现问题，可以先保留 `SessionRuntime` 根门面不变，回滚内部 owner 变更；本次不涉及协议变更，回滚只需恢复模块内调用链。

## Open Questions

- `SessionActor` 是否需要直接暴露 `turn_runtime()` getter，还是应通过更窄的 `runtime_control()` facade 暴露？本次实现可先用直接 getter，后续再视复杂度收窄。
- `session_control_state()` 未来是否应进一步拆成“durable projection snapshot”和“runtime control snapshot”两个结构？本次保持现有返回类型不变，后续按上层需求再评估。
