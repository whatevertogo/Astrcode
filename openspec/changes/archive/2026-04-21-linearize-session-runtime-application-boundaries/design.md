## Context

当前代码的问题不是某几个 Rust 文件太长，而是单 session 真相、应用层编排合同和少量运行时 helper 没有形成清晰的单向链路。

实际代码里已经能看到三类结构性症状：

- `session-runtime` 内部同类语义重复实现。典型例子包括 turn 终态/summary 投影、`session_id` 规范化、以及部分 query helper 在 `query/service.rs`、`query/turn.rs`、`turn/submit.rs` 等位置重复出现。
- `SessionRuntime` 根门面与 crate 根导出面过宽。`crates/session-runtime/src/lib.rs` 同时承担组合入口、公开方法集合和大量类型 re-export，导致外层很容易直接拿到本应留在 runtime 内部的事实结构。
- `application` 的 port trait 与 `lib.rs` re-export 把 `session-runtime` / `kernel` 具体类型继续向上传递，例如 `ProjectedTurnOutcome`、`TurnTerminalSnapshot`、`AgentObserveSnapshot`、`PendingParentDelivery`，使 anti-corruption layer 名义存在、实际上失效。

进一步按真实代码路径核对，`session-runtime` 当前最明显的 5 条纠缠线是：

1. turn 终态投影重复出现在 `state/projection_registry.rs`、`query/turn.rs` 和 registry rebuild 入口中。
2. `turn/submit.rs` 同时承担提交入口、消息准备、turn finalize、subrun 事件构造与 deferred compact 协调。
3. post-compact 事件序列在 `turn/request.rs`、`turn/compaction_cycle.rs`、`turn/manual_compact.rs` 三处重复组装。
4. `turn` 反向依赖 `query`（例如 `current_turn_messages(session_state)`），同时 `interrupt` 又调用 `submit` 内部 helper，形成子域双向渗透。
5. `state/projection_registry.rs` 同时管理 phase、agent、mode、children、tasks、input queue、turn terminal、recent cache 等多域逻辑，已经成为事实上的上帝对象。

这里还要明确一个容易误判的前提：`session-runtime` 不可能被“纯事件驱动”统一掉。更准确的模型是三层：

- 外层合同：`application` / `server` 消费的纯数据快照与纯数据结果。
- 中间真相：append-only 的 durable event stream，所有投影和恢复都从这里出发。
- 内部执行：只有 runtime 自己可见的可变控制状态与副作用，例如 `CancelToken`、`running` 原子标记、lease、流式 LLM/tool 并发调度。

因此本次 change 的目标不是把所有逻辑都挤进事件溯源，而是把“可纯化的投影世界”和“不可纯化的运行时世界”明确分开：投影与外层合同尽量纯数据、可回放；运行时控制状态留在内部，不泄漏为外层事实模型。

这与 `PROJECT_ARCHITECTURE.md` 中“`application` 只通过稳定 runtime 合同消费会话事实、`session-runtime` 内部 helper 不应外泄”的方向并不冲突，但代码层面还没有真正落地。本次 change 的目标不是大爆炸式重构整个仓库，而是完成第一阶段收敛：先把 `session-runtime -> application` 这条主线拉直。

## Goals / Non-Goals

**Goals:**

- 为 `session-runtime` 内部重复的 orchestration/query helper 指定单一 canonical owner，消除“一类事实多处实现”的状态。
- 把 `session-runtime` 内部最明显的 5 条纠缠线改造成可沿单一主线理解的结构。
- 收口 `session-runtime` 面向编排消费者的公开表面：保留稳定 façade，隐藏低层 helper 与不该暴露的运行时细节。
- 为 `application` 引入 app-owned session orchestration contracts，避免继续把 runtime/kernel 内部快照类型作为公共 port 合同暴露。
- 消除 `application` 对 `normalize_session_id` 等 runtime 路径/helper 的直接调用，把这类规范化收回 runtime 端口内部。
- 同步更新 `PROJECT_ARCHITECTURE.md` 与 OpenSpec，使实现边界与仓库级架构表述一致。

**Non-Goals:**

- 不在本次 change 中完成 `server` 对 `session-runtime` 的全面隔离；`ConversationStreamProjector`、HTTP/SSE conversation surface 的全面收口留到后续 change。
- 不在本次 change 中执行 `core` 全面瘦身；`core` 中运行时算法/I/O 的迁移是后续独立阶段。
- 不引入新的 hooks 平台，也不把 hooks 相关改造并入此 change。
- 不重写 `kernel` 总体结构；只允许做必要的极小配合改动。
- 不在本次 change 中把全部 runtime control state 从 `state/` 彻底搬迁到新的 `turn/runtime.rs`；该方向成立，但跨度过大，留给后续专门 change。
- 不在本次 change 中迁移 `wait_for_turn_terminal_snapshot()` 的等待/观察语义；它暂时保留在 `query/service.rs`，后续独立 change 再决定 watcher / lifecycle observer 的最终归属。

## Decisions

### Decision 1: 本 change 只做第一阶段收敛，不做全仓库大爆炸重构

本次 change 只覆盖两条主线：

1. `session-runtime` 内部重复真相与过宽公开表面的收敛。
2. `application` 对 `session-runtime` / `kernel` 运行时内部类型的 anti-corruption contract 修复。

这样做的原因：

- 这两条主线共享同一根问题：单 session 真相没有沿着稳定合同向上收敛。
- 它们可以在不大规模搬动 `server` 与 `core` 的前提下，先把最常用的执行/查询主线拉直。
- 如果把 `server` 全隔离、`core` 瘦身、hooks 平台一起并入，变更会立刻失去可控性。

替代方案是一次性推进 `session-runtime`、`application`、`server`、`core` 的全量边界修复。该方案虽然“更彻底”，但会同时引入过多 API 断裂与跨 crate 迁移，超出本次 change 的可实施范围，因此不采用。

### Decision 2: turn 终态投影统一为一个 shared canonical projector，增量/回放/重建全部复用

本次 change 明确把 turn terminal projection 收敛为一个实现源，供三类路径共用：

- live append 下的 projection reducer 更新
- query 路径的 replay / fallback
- checkpoint / recovery 下的 rebuild

收敛方式不是“每处都保留一份近似 match 分支”，而是提供统一的 projector/reducer helper，放在 `session-runtime` 内部的共享中立模块中（例如 `state/projections/turn.rs` 或等价位置），再由 projection registry 与 `query` 读取路径共同复用。`query` 继续拥有面对外部的读取 API，但不再拥有独占的投影算法副本。

这样做的原因：

- 终态推断是单一语义，不应该因为“增量 vs 全量”就复制两套逻辑。
- `TurnDone` / `Error` 字段一旦演化，只有一个地方需要跟进。
- 这能直接去掉当前 `query/service` 里“先查缓存、没命中再走另一套投影”的双路径心智负担。

替代方案是让 `ProjectionRegistry::TurnProjection::apply()` 与 `query/turn::replay_turn_projection()` 长期并存，只通过测试保证一致。这种方案维护成本高，且天然容易漏改，不采用。

### Decision 3: `submit.rs` 保留提交入口，但 finalize / subrun 事件构造必须拆出独立模块

`turn/submit.rs` 当前把提交入口、消息准备、turn finalize、subrun finished 摘要提取和 deferred compact 协调揉在一起。本次 change 采用“保留 coordinator，拆走重职责实现”的方案：

- `submit.rs` 只保留提交入口、`TurnCoordinator` 和少量胶水逻辑。
- finalize 持久化、失败持久化、deferred compact 落盘迁到独立 `turn/finalize.rs`（或等价模块）。
- subrun started / finished 的事件构造与摘要提取迁到 `turn/events/subrun.rs`（或等价模块）。

同时去掉 `turn` 对 `query` 的反向依赖：`current_turn_messages(session_state)` 这类当前只是包装投影快照的读取，应下沉为 `SessionState` 的直接读取 API 或 neutral helper，`submit` 不再 import `query::*`。

这样做的原因：

- 这能把一次 turn 的主线重新拉直为：accept -> prepare -> run -> finalize。
- 事件构造与事件持久化从 coordinator 中移走后，`submit.rs` 会从“巨型脚本”回到“编排器”角色。
- 消除 `turn -> query` 反向依赖后，子域边界会清晰很多。

替代方案是维持单文件，只在内部多写几个私有函数。这不能解决跨关注点缠绕，也不能让模块边界更清晰，因此不采用。

### Decision 4: post-compact 事件序列统一由共享 builder 生成

主动 compact、reactive compact、manual compact 当前都会组装同一类 durable 事件序列：`compact_applied`、recent user context digest/messages、file recovery messages。本次 change 统一抽出共享 builder，例如 `turn/compact_events.rs`，由不同调用方只负责提供 trigger、turn 上下文和 compact 结果。

这样做的原因：

- 这是一种典型的“同一语义在三个路径里复制”的问题，适合直接抽成共享 builder。
- compact 事件序列对恢复与展示都很关键，不应允许三个调用点长期各自维护。
- 该 builder 天然可单测，能直接降低 manual/reactive/proactive 三条路径的回归成本。

替代方案是继续在三个调用点各自组装，只靠 review 保持一致。这种方案在事件模型演化时极易漏改，因此不采用。

### Decision 5: `ProjectionRegistry` 保留为薄协调器，但各投影域拆成独立 reducer

本次 change 不直接删除 `ProjectionRegistry`，而是把它降成薄协调器：

- `agent` / `phase` / `mode`
- `children`
- `tasks`
- `input_queue`
- `turn_terminal`
- `recent_cache`

每个域各自拥有 `apply` / `snapshot` / `rebuild` 逻辑，registry 只负责固定顺序委托，不再自己堆满跨域细节。类似 `upsert_child_session_node` 这种命令式后门，如果短期不能删除，也应被收敛到对应 reducer 内部，而不是继续挂在 registry 根对象上。

这样做的原因：

- 这能明显降低“改一个投影域，必须同时碰 registry 分发、重建逻辑和局部函数”的编辑半径。
- registry 仍然保留统一入口，避免引入第二套旁路。
- 它与本次 turn projector 收敛、app-owned contracts 改造是同方向的收敛动作。

替代方案是彻底删除 registry，让调用方分别维护各投影。那会造成更多旁路与一致性问题，因此不采用。

### Decision 6: `state/` 与 `turn/` 边界本次只做“去反向依赖 + 去命令污染”，不做彻底搬家
glm 的判断“`state/` 和 `turn/` 边界画错了”是对的，但本次只采纳其中风险较低、收益更高的部分：

- 采纳：去掉 `turn -> query` 的反向依赖。
- 采纳：把 `InputQueueEventAppend` / `append_input_queue_event` 这类命令语义从 `state` 的边缘收紧到 `command` 所拥有的调用路径。
- 采纳：把只读的 transcript/session replay API 从 `turn/replay.rs` 迁回 `query` 子域。
- 延后：把 `TurnRuntimeState` / `CompactRuntimeState` 整体从 `state` 迁到 `turn/runtime.rs`。

这样做的原因：

- 现在最紧的是把重复、反向依赖和上帝对象打散，而不是先触碰 `SessionState` 的大面积持有关系。
- 彻底搬 runtime control state 会牵动 actor、interrupt、submit、query 与大量测试，适合作为后续专门 change。
- 先做“边界收口 + reducer 化 + coordinator 拆责”，已经能显著改善理解成本。

替代方案是本次就把 `state/` 与 `turn/` 做彻底搬家。这个方向长期成立，但实现半径过大，不适合并进第一阶段，因此不采用。

### Decision 7: 外层合同保持纯数据，运行时控制状态继续留在内部

这次 change 明确采用“三层分离”的约束：

- `application` / `server` 所消费的 session facts 与 orchestration contracts 必须是纯数据 DTO / snapshot。
- durable event stream 继续作为中间真相来源，投影与恢复统一从事件出发。
- `CancelToken`、`AtomicBool(running)`、active turn generation、lease、流式调度状态等运行时控制信息继续留在 runtime 内部，不作为外层合同泄漏。

这样做的原因：

- agent 系统的投影侧可以也应该高度事件驱动，但运行时并发控制本质上不是可回放投影。
- 如果把运行时控制状态也伪装成外层事实合同，`application` 和 `server` 会开始理解本不该理解的并发/取消语义。
- 外层只拿纯数据快照，才能把 anti-corruption layer 做实。

替代方案是把 runtime control state 也包装成正式公共合同，或者尝试用“纯事件驱动”统一取消、running flag 与并发调度。这会混淆 durable truth 和 process-local control，不采用。

### Decision 8: 所有跨出 runtime 的扩展点都遵循“收纯数据、吐纯数据”

这条规则不只适用于 `application` 的 orchestration contracts，也适用于一切跨出 runtime 边界的扩展点：

- 订阅 / stream payload：输出纯数据事件或纯数据 snapshot
- hook 输入输出：输出纯数据上下文与纯数据决策
- capability / tool 注册：声明侧与执行结果侧都以纯数据 DTO 表达
- policy 输入输出：通过纯数据 `context -> verdict` 交互
- plugin / manifest 注入：通过纯数据声明注册，不暴露 runtime 内脏

当前代码里已经有一些正确样例：

- `SessionEventRecord` 作为事件订阅载荷
- `astrcode_core::HookInput` / `HookOutcome`
- `astrcode_core::PolicyContext` / `PolicyVerdict`
- `astrcode_core::CapabilitySpec`

本次 change 不会实现新的 hooks / plugin 平台，但会把这条规则写成今后 session-runtime 边界整理的硬约束：外部扩展点只接触数据，不接触 `CancelToken`、锁、原子变量、active turn 句柄等 process-local runtime state。

这里要区分两种“句柄”：

- runtime-local 组合细节：例如 server/application 组合期内部使用的 receiver / handle，不属于对外扩展合同
- cross-boundary contracts：真正跨 runtime 边界暴露给上层、插件、hook、policy 或远端消费者的输入输出，必须保持纯数据

替代方案是让外部扩展点直接持有 runtime handle 或控制状态，换取“更方便地介入执行”。这会把 runtime 内脏扩散到系统各处，长期不可维护，不采用。

### Decision 9: `SessionRuntime` 继续保留根 façade，但 crate 根导出面必须收口

`SessionRuntime` 仍然是外部消费单 session 能力的主入口；本次不把它拆成多个公开对象，也不新增独立 crate。  
但要收紧两件事：

- 根对象的方法继续按 query / command / orchestration 进行内部委托，避免根实现继续膨胀。
- crate 根的 `pub use` 只保留稳定快照、稳定 read-model facts 和确实需要跨 crate 暴露的结果类型；低层 helper、路径规范化函数、执行辅助类型不再继续作为默认导出面。

这一决策意味着：

- `session-runtime` 的“公开对象”保持稳定，降低改动面。
- “哪些东西能被外层拿到”这件事会被显式收紧，避免外层继续通过 crate 根顺手越界。

替代方案是把 `SessionRuntime` 整体拆成 `SessionQueries` / `SessionCommands` / `TurnEngine` 三个公开服务对象。这种拆法最终可能是合理方向，但会显著放大本次 API 断裂，因此暂不采用。

### Decision 10: `application` 为 orchestration-only session facts 定义 app-owned contracts

本次只把“用于应用编排”的 session facts 收到 `application` 自己的合同里，而不是把所有 runtime read model 一次性搬完。拟新增 `application::ports::session_contracts`（名称可微调）承载 app-owned DTO，例如：

- turn 终态等待结果
- turn outcome 摘要
- observe 摘要
- recoverable parent delivery 摘要

这些类型由 `application` 定义、由 `SessionRuntime` blanket impl 负责映射填充。这样：

- `AgentSessionPort` / `AppSessionPort` 不再直接暴露 `ProjectedTurnOutcome`、`TurnTerminalSnapshot`、`AgentObserveSnapshot`、`PendingParentDelivery`。
- `application/lib.rs` 不再 re-export 这些 runtime/kernel 内部结构。
- `application` 以自己的语言描述“编排需要知道什么”，而不是继续承接 runtime 内部事实模型。

这里故意**不**在本次 change 中处理全部 terminal conversation facts。`ConversationSnapshotFacts` / `ConversationStreamReplayFacts` 这类更接近终端 authoritative read model 的合同，留到后续 `server` 隔离 change 处理。

推荐的 contract 对照如下：

| App-Owned Contract | 替代的 Runtime/Kernel 类型 | 关键字段 |
| --- | --- | --- |
| `AppTurnOutcome` | `ProjectedTurnOutcome` | `outcome`, `summary`, `technical_message` |
| `AppTurnTerminalSnapshot` | `TurnTerminalSnapshot` | `phase`, `projection`, `events` |
| `AppAgentObserveSnapshot` | `AgentObserveSnapshot` | `phase`, `turn_count`, `active_task`, `last_turn_tail` |
| `AppParentDeliverySummary` | `PendingParentDelivery` | `delivery_id`, `parent_agent_id`, `payload` 摘要、来源语义 |

命名可以微调，但本次 change 的实现必须提供一一对应的 app-owned contract，而不是再让实现者自行猜字段边界。

替代方案是把所有 `session-runtime` 暴露类型一次性包装成 app-owned DTO。该方案过重，会把本次 change 拉成第二个 transport 层，不采用。

### Decision 11: 输入规范化留在 runtime 端口内部，`application` 不再直接调用 runtime helper

当前 `application` 中存在直接调用 `astrcode_session_runtime::normalize_session_id(...)` 的代码。这会让应用用例代码知道 runtime 的路径/标识规范化细节，边界已经破了。

本次改为：

- `application` 把外部输入当作原始字符串处理。
- `AppSessionPort` / `AgentSessionPort` 的实现内部负责规范化与 typed conversion。
- 若 `application` 自己需要长期复用输入校验逻辑，只保留 app-owned 的“字段不能为空/格式非法”检查，不复用 runtime helper。

替代方案是在 `application` 再复制一套 `normalize_session_id`。这只会制造第二个 canonical owner，因此不采用。

### Decision 12: `PROJECT_ARCHITECTURE.md` 需要同步补强，但不改总体原则

本次 change 不改变现有仓库级架构原则；`PROJECT_ARCHITECTURE.md` 的总体方向已经正确。  
需要补强的是两点表述：

- `application` 依赖的是稳定 runtime 合同，而不是 runtime 的内部快照与 helper。
- `session-runtime` 的内部 helper、执行辅助和路径规范化不属于外层合同。

因此本次需要同步更新 `PROJECT_ARCHITECTURE.md`，但属于“表述与代码重新对齐”，不是架构原则翻案。

## Files

**新增文件：**

- `crates/application/src/ports/session_contracts.rs`
  - 原因：为 `application` 定义 app-owned session orchestration contracts，避免 port trait 继续泄漏 runtime/kernel 内部类型。

**重点修改文件：**

- `crates/session-runtime/src/lib.rs`
  - 原因：收口 crate 根导出面，减少对低层 helper/路径工具的外泄。
- `crates/session-runtime/src/query/replay.rs`（或等价新文件）
  - 原因：承接 `turn/replay.rs` 中只读的 transcript/session replay 逻辑，消除只读查询留在 `turn/` 的错位。
- `crates/session-runtime/src/query/turn.rs`
  - 原因：保留 turn 读取 API，并复用共享 turn projector / summary helper。
- `crates/session-runtime/src/query/service.rs`
  - 原因：复用 canonical helper，删除局部重复实现。
- `crates/session-runtime/src/turn/submit.rs`
  - 原因：拆出 finalize / subrun 事件构造职责，并移除对 `query` 的反向依赖。
- `crates/session-runtime/src/turn/finalize.rs`（或等价新文件）
  - 原因：承接 finalize、失败持久化与 deferred compact 落盘逻辑。
- `crates/session-runtime/src/turn/events/subrun.rs`（或等价新文件）
  - 原因：承接 subrun started / finished 事件构造与摘要提取逻辑。
- `crates/session-runtime/src/turn/compact_events.rs`（或等价新文件）
  - 原因：统一主动 / 被动 / 手动 compact 后的 durable 事件序列构造。
- `crates/session-runtime/src/command/mod.rs`（或 `src/command/input_queue.rs`）
  - 原因：承接 `InputQueueEventAppend` / `append_input_queue_event` 的命令语义，避免 `state/input_queue.rs` 混杂写路径。
- `crates/session-runtime/src/state/paths.rs`
  - 原因：成为 `session_id` 规范化的唯一所有者。
- `crates/session-runtime/src/state/projection_registry.rs`
  - 原因：降级为薄协调器，并把 turn / children / tasks / input queue 等投影域的 reducer 逻辑拆开。
- `crates/application/src/ports/app_session.rs`
  - 原因：切换到 app-owned session contracts，收紧 blanket impl 边界。
- `crates/application/src/ports/agent_session.rs`
  - 原因：去除 `ProjectedTurnOutcome`、`TurnTerminalSnapshot`、`AgentObserveSnapshot`、`PendingParentDelivery` 等内部类型泄漏。
- `crates/application/src/lib.rs`
  - 原因：删除仅服务于编排内部的 runtime re-export，保留必要稳定 surface。
- `crates/application/src/agent/context.rs`
- `crates/application/src/agent/wake.rs`
- `crates/application/src/session_use_cases.rs`
  - 原因：移除对 `normalize_session_id` 等 runtime helper 的直接依赖。
- `PROJECT_ARCHITECTURE.md`
  - 原因：同步补强稳定 runtime 合同与内部 helper 不外泄的边界表述。

**可能删除的导出：**

- `crates/application/src/lib.rs` 中仅供 orchestration 使用的 `session-runtime` re-export。
- `crates/session-runtime/src/lib.rs` 中不该作为外层默认表面的路径/执行辅助导出。

## Risks / Trade-offs

- [风险] 端口合同改动会触发较多编译级联修改。  
  → 缓解：先引入 app-owned contracts，再通过 blanket impl 一次性替换调用点；同一 change 内删除旧 re-export，避免长期双轨。

- [风险] “只做第一阶段”会暂时保留部分不干净边界，例如 terminal read model 相关 runtime 类型仍存在。  
  → 缓解：在 design 与 tasks 中明确这是刻意保留的后续切片，不让其继续扩散到新的编排合同。

- [风险] `submit.rs` 拆分会同时影响执行路径和测试夹具。  
  → 缓解：先保持 `TurnCoordinator` 对外入口稳定，只移动 finalize / subrun 事件构造等重职责逻辑；每次拆分后立即跑相邻测试。

- [风险] ProjectionRegistry reducer 化可能引入恢复路径与 live append 路径不一致。  
  → 缓解：要求每个 reducer 同时暴露 `apply` 与 `rebuild`，并补 recovery/live 等价测试。

- [风险] 将 canonical owner 收到 `query/turn` 后，若命名或模块切分不清晰，可能只是把重复逻辑换个地方堆。  
  → 缓解：限制本次改动只引入少量 helper，并要求 `query/service` / `turn/submit` 只复用，不再自行派生。

- [风险] 收口 `normalize_session_id` 可能影响现有宽松输入兼容。  
  → 缓解：保留 runtime 内部规范化语义不变，只改变调用位置与所有权；为关键入口补回归测试。

- [风险] 文档与实现不同步，导致后续 change 仍按旧习惯继续泄漏。  
  → 缓解：本次同步更新 `PROJECT_ARCHITECTURE.md`，并在 tasks 中加入边界检查与 grep 验证。

## Migration Plan

1. 先在 `application` 引入 app-owned session contracts，并为 `AppSessionPort` / `AgentSessionPort` 增加映射。
2. 修改 `application` 调用点，移除对 runtime helper 和 runtime internal types 的直接依赖。
3. 在 `session-runtime` 内统一 shared turn projector / summary helper，并让 `query/service`、`query/turn`、`turn/submit`、projection registry 共用。
4. 拆出 `submit` 的 finalize / subrun 事件构造职责，并统一 compact 后事件 builder。
5. 将 transcript/session replay 的只读 API 迁回 `query` 子域，并把 input-queue 命令语义迁回 `command` 子域。
6. 将 `ProjectionRegistry` 降成薄协调器，提炼 turn/children/tasks/input_queue 等 reducer。
7. 收口 `lib.rs` 导出面，删除已经无人使用的 runtime/application re-export。
8. 更新 `PROJECT_ARCHITECTURE.md`、OpenSpec 与回归测试。

回滚策略：

- 若中途发现 contract 改动影响面超出预期，可保留 app-owned contract 模块但暂不删除旧 re-export，先完成内部映射与调用点迁移，再在下一次提交中删除兼容层。
- 不进行持久化 schema 变更，因此不存在数据回滚问题；回滚主要是源码级 API 回退。

## Open Questions

- `ConversationSnapshotFacts` / `ConversationStreamReplayFacts` 是否在后续 change 中一起迁入 application-owned terminal contracts，还是继续保留为 runtime-owned authoritative read model facts？
- `CapabilityRouter` 出现在 `application` 公共 API 的问题是否与本次 change 同步处理，还是留给 agent-kernel boundary 的下一阶段？
- `SessionCatalogEvent` 是否也应在后续进入 application-owned contract，而不是继续由 runtime 直接暴露？
- `TurnRuntimeState` / `CompactRuntimeState` 是否在下一阶段整体迁往 `turn/runtime.rs`，并把 `state` 收窄成“writer + projection + cache”纯事实子域？
- `wait_for_turn_terminal_snapshot()` 这类带异步轮询/等待语义的能力，是否应在下一阶段从 `query/service` 迁往更明确的 watcher / turn-lifecycle observer，而不是继续挂在纯 query 语义名下？
