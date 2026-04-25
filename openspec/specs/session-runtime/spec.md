## Requirements

### Requirement: `runtime-session` / `runtime-agent-loop` / `runtime-execution` 迁入 `session-runtime`

以下旧层能力 SHALL 迁入 `session-runtime`：

- `runtime-session` -> `session-runtime/state`
- `runtime-agent-loop` -> `session-runtime/turn`
- `runtime-execution` -> `session-runtime/actor` 与 `session-runtime/context_window`
- `runtime/service/session/*` -> `session-runtime/catalog`
- `runtime/service/turn/*` 与 `runtime/service/agent/*` -> `session-runtime` 对应子模块

#### Scenario: 旧会话执行层最终删除

- **WHEN** 清理阶段完成
- **THEN** workspace 不再包含 `runtime-session`、`runtime-agent-loop`、`runtime-execution`

---

### Requirement: durable append 是执行主路径职责

`session-runtime` SHALL 持有 `Arc<dyn EventStore>`，并将 durable append 作为执行主路径的一部分。

#### Scenario: 关键事件先落盘再推进

- **WHEN** SessionActor 追加关键事件
- **THEN** 通过 `EventStore::append()` 落盘
- **AND** 再继续后续执行步骤

---

### Requirement: SessionActor 通过 kernel 间接调用 provider

SessionActor SHALL NOT 直接持有 `LlmProvider`、`PromptProvider`、`ToolProvider`、`ResourceProvider`。

#### Scenario: SessionActor 字段干净

- **WHEN** 检查 `SessionActor` 字段
- **THEN** 不存在上述 provider 直接字段
- **AND** provider 调用由 `kernel` gateway 承担

---

### Requirement: 协作执行桥接实现归 `session-runtime`

`SubAgentExecutor` 与 `CollaborationExecutor` 的实际执行桥接 SHALL 位于 `session-runtime`。

#### Scenario: runtime 旧门面不再承载协作桥接

- **WHEN** 清理阶段完成
- **THEN** 旧 `runtime` crate 中不再保留这两类桥接实现

---

### Requirement: 公共 API 使用强类型并隐藏并发容器

`session-runtime` 公共 API SHALL 使用 `SessionId`、`TurnId`、`AgentId` 等强类型；内部并发容器 SHALL NOT 外泄。

#### Scenario: 外部获取 handle 或 snapshot

- **WHEN** `application` 或 `server` 查询会话状态
- **THEN** 返回 typed handle / snapshot / query result（如 `SessionObserveSnapshot`, `SessionSnapshot`, `ConversationSnapshotFacts` 等）
- **AND** 不返回内部 `DashMap` 或锁对象

---

### Requirement: `session-runtime` 内部继续按单 session 职责分块

`session-runtime` 内部 SHALL 至少按以下职责分块组织，而不是把所有执行细节平铺在 crate 根：

- `state` — durable projection state、事件投影、child session 节点跟踪、input queue 投影、writer 与广播基础设施
- `catalog` — session catalog 事件 re-export 与广播协调
- `actor` — 单 session live truth 组装与 `SessionState` / `TurnRuntimeState` owner
- `turn` — turn 用例、执行核心、runtime control state 与 turn watcher（submit, interrupt, branch, fork, runner, request, runtime, watcher 等）
- `context_window` — token 预算、裁剪、压缩与窗口化消息序列
- `command` — 写操作 façade（append 各种 durable 事件、compact、switch mode 等）
- `query` — 纯读 façade（observe 所需快照、conversation snapshot、replay、transcript、turn terminal snapshot 等）
- `observe` — observe/replay/live 订阅语义、scope/filter 与状态来源
- `heuristics` — 运行时启发式常量（token 估算等）

其中子域职责 MUST 满足以下约束：

- `context_window` 只负责预算、裁剪、压缩与窗口化消息序列
- request assembly 位于 `turn/request`，不在 `context_window` 名下
- `actor` 只负责组装与持有单 session live truth，不承担 query 或 watcher 语义
- `observe` 只负责推送/订阅语义与过滤范围
- `query` 只负责拉取、快照与回放，不负责订阅等待循环或 turn 运行时协调
- `command` 只负责写操作与 durable event append
- `state` 包含 cache, child_sessions, execution, input_queue, paths, tasks, writer 等 durable/projection 子模块
- `turn` 包含 runtime control、watcher 与完整执行循环；`TurnRuntimeState` 等运行时控制类型 MUST 归属 `turn`

#### Scenario: 单 session 真相与执行结构清晰

- **WHEN** 检查 `session-runtime/src`
- **THEN** 可以沿着 `state -> actor -> turn -> query` 的结构理解单 session 行为
- **AND** 不需要在 `state` 中同时追踪 turn runtime control 与 durable projection truth

#### Scenario: request assembly 不再挂在 context_window 名下

- **WHEN** 检查 `context_window` 子域
- **THEN** 其中只保留预算、裁剪、压缩与窗口化逻辑
- **AND** 最终 request assembly 位于 `turn/request`

#### Scenario: query 按读取语义拆分子模块

- **WHEN** 检查 `query` 子域
- **THEN** 其实现按 `agent`, `conversation`, `input_queue`, `service`, `terminal`, `text`, `transcript`, `turn` 等读取场景拆分
- **AND** crate 根只保留统一入口与类型导出

#### Scenario: turn 包含完整的执行循环

- **WHEN** 检查 `turn` 子域
- **THEN** 包含 `runner`（step 循环）、`submit`、`replay`、`interrupt`、`branch`、`fork`、`request`、
  `llm_cycle`、`tool_cycle`、`continuation_cycle`、`compaction_cycle`、`loop_control`、`events`、
  `summary`、`tool_result_budget` 等子模块

#### Scenario: state 包含完整的状态管理子模块

- **WHEN** 检查 `state` 子域
- **THEN** 包含 `cache`, `child_sessions`, `execution`, `input_queue`, `paths`, `tasks`, `writer` 等子模块
- **AND** 公共导出包括 `SessionSnapshot`, `SessionState`, `append_and_broadcast`, `complete_session_execution`,
  `display_name_from_working_dir`, `normalize_session_id`, `normalize_working_dir`, `prepare_session_execution`

#### Scenario: state 不再拥有 turn runtime control 类型

- **WHEN** 检查 `state` 子域
- **THEN** 其中不再定义 `TurnRuntimeState`、`CompactRuntimeState`、`ActiveTurnState`、`ForcedTurnCompletion` 或 `PendingManualCompactRequest`
- **AND** 这些类型 SHALL 归属 `turn/runtime.rs` 或等价的 turn-owned 模块

#### Scenario: query 保持纯读与回放语义

- **WHEN** 检查 `query` 子域
- **THEN** 其实现只包含 snapshot、projection、replay、transcript 与等价的纯读能力
- **AND** 不再包含 `wait_for_turn_terminal_snapshot()` 这类基于 broadcaster 的等待循环

#### Scenario: turn 拥有 watcher 与 runtime control

- **WHEN** 检查 `turn` 子域
- **THEN** 其实现包含 `runtime` 和 `watcher`（或等价命名）的子模块
- **AND** turn terminal 等待语义 SHALL 由 `turn` 子域拥有

### Requirement: `session-runtime` SHALL 分离 runtime control state 与 display projection state

`session-runtime` MUST 把“执行控制状态”和“面向读模型的 display phase / projected state”建模为两类不同真相。runtime control state 用于持有 active turn、cancel、lease 与 compacting 等控制信息；display projection state 继续由 durable 事件流投影得到。

运行时控制状态的模块 owner SHALL 位于 `turn` 子域；`SessionState` SHALL 只承载 durable projection state 与相关基础设施，不再直接拥有 runtime control state。

#### Scenario: turn 提交更新 runtime control state 而不是直接声明 display phase 真相

- **WHEN** 系统开始一个新的 turn
- **THEN** `session-runtime` SHALL 先更新内部 runtime control state 以记录 active turn、cancel token 与 lease
- **AND** display phase 的长期可恢复真相仍 SHALL 通过 durable 事件投影到 read model

#### Scenario: reload 后 display phase 仍从 durable 事件恢复

- **WHEN** 一个 session 从 durable 历史冷恢复
- **THEN** 系统 SHALL 从事件投影恢复 display phase
- **AND** SHALL NOT 依赖进程内残留的 runtime control state 判断该 session 的最终展示状态

#### Scenario: TurnRuntimeStage 与 display Phase 只保持最终一致，而非同步写入

- **WHEN** `TurnRuntimeStage` 发生变更（如从 `Preparing` 进入 `RunningModel`）
- **THEN** 系统 SHALL 把该 stage 变更只视为 runtime control 语义
- **AND** display `Phase` SHALL 继续只由 durable events 经 `PhaseTracker` 投影得到
- **AND** 设计中的 stage->phase 映射 SHALL 只表示“正常事件流完成后 display Phase 最终会收敛到哪里”
- **AND** SHALL NOT 在 stage 变更时直接 `phase.lock()` 或等价方式同步设置 display Phase

#### Scenario: running 标志作为 active turn 的 lock-free 缓存镜像

- **WHEN** `TurnRuntimeState` 的 `prepare()` 或 `complete()` 方法被调用
- **THEN** 系统 SHALL 同步更新一个 lock-free `running` 原子布尔，使其始终镜像 `active_turn.is_some()` 的结果
- **AND** 外部消费者（如 `list_running_sessions`）SHALL 通过该原子布尔读取，而不是 acquire mutex
- **AND** 该原子布尔 SHALL NOT 被视为独立真相，其不变式为 `running.load() == active_turn.is_some()`

#### Scenario: CompactRuntimeState 收敛 deferred compact 控制字段

- **WHEN** 系统维护 compacting、pending manual compact 与 compact failure count
- **THEN** 它们 SHALL 收敛到 `CompactRuntimeState`
- **AND** `CompactRuntimeState` SHALL 至少持有 `in_progress`、`failure_count` 与 `pending_request`
- **AND** SHALL 使用 `pending_request.is_some()` 作为唯一“存在待执行 deferred compact”的真相
- **AND** SHALL NOT 再并行维护单独的 `pending_manual_compact: bool`

#### Scenario: SessionState 不再直接拥有 runtime control state

- **WHEN** 检查 `SessionState` 结构
- **THEN** 其字段只包含 projection registry、writer、broadcaster 与等价的 durable/projection 基础设施
- **AND** `TurnRuntimeState` SHALL 由 `turn` 子域定义并由单 session live truth owner 单独持有

#### Scenario: prepare / complete / interrupt 只维护 runtime control，不直接写 display Phase

- **WHEN** `TurnRuntimeState::prepare()`、`complete()` 或 `interrupt_if_running()` 被调用
- **THEN** 系统 SHALL 只更新 active turn、generation、cancel、compacting 与 running 等 runtime control 字段
- **AND** display `Phase` SHALL 继续只由 durable events 经 `PhaseTracker` 投影得到
- **AND** SHALL NOT 在这些 runtime control transition 中直接 `phase.lock()` 或等价方式同步设置 display Phase

### Requirement: `session-runtime` SHALL 通过统一 projection registry 增量维护派生事实

`session-runtime` MUST 使用统一的 projection registry 增量维护 session 派生事实，包括至少：phase tracker、agent projection、mode projection、turn projection、child session projection、active task projection 与 input queue projection。追加一条 stored event 后，所有这些派生事实 SHALL 通过统一入口更新。

#### Scenario: live append 更新所有相关 projections

- **WHEN** session 成功追加一条新的 stored event
- **THEN** `session-runtime` SHALL 通过统一 projection registry 更新相关派生事实
- **AND** SHALL NOT 依赖多个分散的条件分支在不同位置手动维护同一类投影

#### Scenario: recovery replay 与 live append 产出一致的投影结果

- **WHEN** 系统分别通过 recovery replay 和 live append 处理等价的 stored event 序列
- **THEN** phase、mode、turn terminal、child session、active tasks 与 input queue 的投影结果 SHALL 保持一致
- **AND** query 路径读取到的 authoritative facts SHALL 不因处理路径不同而漂移

#### Scenario: ProjectionRegistry 采用 stateful reducer 协议

- **WHEN** `ProjectionRegistry` 应用一条 stored event
- **THEN** 每个 reducer SHALL 以有状态对象的形式通过统一的 `apply(event, effects)` 协议运行
- **AND** reducer 之间 SHALL 只共享 `StoredEvent` 与统一 `ProjectionEffects`
- **AND** reducer 的应用顺序 SHALL 固定且可审计，至少包含 `phase_tracker -> agent_projection -> mode_projection -> turn_projection`

#### Scenario: PhaseTracker 作为 ProjectionRegistry 的一等 reducer

- **WHEN** `ProjectionRegistry` 被构建
- **THEN** `PhaseTracker` SHALL 被纳入 registry，而不是作为独立于 registry 的第二套 phase 真相
- **AND** `PhaseTracker` 在必要时 MAY 通过 `ProjectionEffects` 产出 live `AgentEvent`
- **AND** 这类 side effect SHALL 由 registry 统一收集和转发，而不是在 reducer 外侧额外维护旁路逻辑

### Requirement: `session-runtime` SHALL 将事件追加与投影广播归为 SessionState 方法

`session-runtime` MUST 将当前作为 free function 的 `append_and_broadcast` 收为 `SessionState` 的方法，使其内部依次执行：写入 event log -> `projection_registry.apply(stored)` -> `translator.translate(stored)` -> 广播 records。该重构 SHALL 与 `ProjectionRegistry` 引入同步完成。

#### Scenario: append_and_broadcast 成为 SessionState 方法

- **WHEN** 任意路径需要追加事件并广播
- **THEN** 系统 SHALL 通过 `SessionState` 方法统一完成写入、投影、翻译和广播
- **AND** SHALL NOT 在外部通过 free function 绕过 projection registry

### Requirement: `SessionRecoveryCheckpoint` SHALL 演化为 projection registry 快照

`SessionRecoveryCheckpoint` MUST 从“平铺的一组 ad-hoc 顶层字段”演化为“agent projection + projection registry snapshot”的结构，避免 checkpoint 成为第二套投影真相。

#### Scenario: 新 checkpoint 不再平铺 phase 和 mode 时间戳

- **WHEN** 系统写入新的 `SessionRecoveryCheckpoint`
- **THEN** 顶层 `phase` 字段 SHALL 被移除
- **AND** 顶层 `last_mode_changed_at` 字段 SHALL 被移除
- **AND** display phase 与 mode 时间戳 SHALL 通过 projection 快照恢复

#### Scenario: 旧 checkpoint 可被兼容恢复

- **WHEN** 系统加载旧版本 checkpoint，且其中缺失 `projection_registry` 快照
- **THEN** 恢复路径 SHALL 能从旧顶层字段构造等价的新 projection snapshot
- **AND** 新写入路径 SHALL 只写新 schema

### Requirement: `session-runtime` SHALL 在 turn 完成时原子清理控制状态并取出 deferred compact

`TurnRuntimeState::complete()` MUST 在单次调用中完成：设置 terminal runtime state、清理 active turn / cancel / lease、并原子取出 pending manual compact request。调用方 SHALL NOT 在 `complete()` 之外再单独调用 `take_pending_manual_compact`。

#### Scenario: complete 原子返回 pending manual compact request

- **WHEN** turn 正常完成或异常终止
- **THEN** `TurnRuntimeState::complete()` SHALL 返回 `Option<PendingManualCompactRequest>`
- **AND** 调用方 SHALL 基于该返回值决定是否执行 deferred compact
- **AND** SHALL NOT 在 `complete()` 之后再通过单独方法读取 compact 状态

### Requirement: turn SHALL 通过 typed lifecycle coordinator 推进，而不是由多模块分段拼装

当前一次 turn 的生命周期散落在 `session_use_cases.rs`（accept）、`submit.rs`（prepare + spawn）、`runner.rs`（run）、`execution.rs`（prepare/complete helper）、`submit.rs finalize`（persist + finalize + deferred compact）之间。系统 MUST 引入显式 `TurnCoordinator` 协议，把 `accept -> prepare -> run -> persist -> finalize -> deferred_compact` 收为单一协调器的生命周期方法，而不是由多个模块各自持有部分状态和逻辑。

#### Scenario: TurnCoordinator 封装完整 turn 生命周期

- **WHEN** `SessionRuntime` 接受一次 turn 提交
- **THEN** 系统 SHALL 通过 `TurnCoordinator` 的生命周期方法依次推进：`accept` -> `prepare` -> `run` -> `persist` -> `finalize`
- **AND** 每个 phase 变更 SHALL 通过 `TurnRuntimeState` 的 typed transition API 触发
- **AND** `finalize` 内部 SHALL 原子执行 `TurnRuntimeState::complete()` 并基于其返回值决定是否触发 deferred compact

#### Scenario: TurnCoordinator 为 per-turn 具体对象

- **WHEN** `submit.rs` 接受一次新的 turn 提交
- **THEN** 它 SHALL 为该 turn 构造一个短生命周期的具体 `TurnCoordinator`
- **AND** 该 coordinator SHALL 在 turn 结束后释放
- **AND** SHALL NOT 被注册为 `SessionActor` 的长期状态对象

#### Scenario: submit.rs 不再直接持有 prepare/run/finalize 分段逻辑

- **WHEN** `TurnCoordinator` 被引入后
- **THEN** `submit.rs` SHALL 只负责解析请求并调用 `TurnCoordinator::start()`
- **AND** SHALL NOT 直接操作 `phase.lock()`、`prepare_session_execution()` 或 `complete_session_execution()`
- **AND** `runner.rs` SHALL 保持为纯 step 循环执行器，不承担生命周期编排

### Requirement: turn 终态 SHALL 使用 typed TurnTerminalKind，查询侧通过 TurnProjection 获取终态

当前 turn 终态语义通过字符串约定传递：`TurnStopCause` 先转字符串，写入 `TurnDone.reason`，查询侧再靠字符串匹配和 `Phase::Interrupted` 反推结果。系统 MUST 在 `core` 引入 typed `TurnTerminalKind`，并扩展 `ProjectionRegistry` 包含 `TurnProjection`，让 `wait_for_turn_terminal_snapshot()` 等待投影终态而不是扫描事件做启发式判断。

#### Scenario: TurnDone 以兼容 schema 携带 typed terminal kind

- **WHEN** turn 到达终态并写入 `TurnDone`
- **THEN** 新 schema SHALL 至少包含 `timestamp`、可选的 `terminal_kind` 与兼容字段 `reason`
- **AND** 新写入路径 SHALL 写入 `terminal_kind`
- **AND** 反序列化旧事件时，系统 SHALL 优先读取 `terminal_kind`，若其缺失再通过 legacy `reason` 映射恢复 typed terminal kind

#### Scenario: 旧 reason 不被误解释为 Error{message}

- **WHEN** 系统反序列化只包含 legacy `reason` 的旧 `TurnDone`
- **THEN** 已知 canonical reason code SHALL 映射到对应 typed terminal kind
- **AND** 任意未知自由文本 SHALL NOT 直接映射为 `TurnTerminalKind::Error { message }`
- **AND** error message SHALL 只来自 typed `terminal_kind` 或相邻 `Error` event

#### Scenario: TurnProjection 扩展 ProjectionRegistry

- **WHEN** `ProjectionRegistry` 处理 `TurnDone` 事件
- **THEN** 系统 SHALL 通过 `TurnProjection` 记录该 turn 的 `TurnTerminalKind` 和摘要信息
- **AND** `wait_for_turn_terminal_snapshot()` SHALL 等待 `TurnProjection` 到达终态
- **AND** SHALL NOT 通过扫描 `TurnDone` 事件列表做启发式判断

#### Scenario: turn 终态 enum 收敛为 durable truth + runtime cause 两层

- **WHEN** typed terminal migration 完成
- **THEN** `TurnTerminalKind` SHALL 成为 durable/query 终态真相
- **AND** `TurnStopCause` SHALL 只保留为 runtime 内部 loop 决策原因
- **AND** `TurnOutcome` 和 `TurnFinishReason` SHALL 被移除或降级为从 `TurnTerminalKind` 派生的视图

### Requirement: step 收到无工具输出后 SHALL 经过统一 PostLlmDecisionPolicy 决策

当前“LLM 返回纯文本（无 tool calls）后下一步怎么办”的逻辑分裂在 `continuation_cycle.rs`（输出截断 continuation）、`loop_control.rs`（budget auto-continue）、`step/mod.rs`（turn done）三处，靠执行顺序隐式耦合。系统 MUST 引入统一 `PostLlmDecisionPolicy`，在 step 收到无工具输出后返回 typed 决策：`ContinueWithPrompt` / `Stop(TurnStopCause)` / `ExecuteTools` 之一，使 agent loop 成为可读的决策表。

#### Scenario: 无工具输出经单一决策层裁决

- **WHEN** step 收到 LLM 输出且该输出不包含 tool calls
- **THEN** 系统 SHALL 将输出送入 `PostLlmDecisionPolicy`
- **AND** 该 policy SHALL 综合考虑：输出截断状态、budget 余量、continuation 计数、step 限制
- **AND** SHALL 返回 `ContinueWithPrompt`、`Stop(TurnStopCause)` 或 `ExecuteTools` 之一
- **AND** SHALL NOT 让 continuation_cycle、loop_control、step 三者通过执行顺序隐式决定最终行为

#### Scenario: 决策表可被独立测试

- **WHEN** `PostLlmDecisionPolicy` 被独立调用
- **THEN** 给定固定的 LLM 输出、step 状态和 runtime 配置
- **AND** 系统 SHALL 返回确定性的决策结果
- **AND** 该结果 SHALL 与完整 turn loop 中的实际行为一致

### Requirement: turn 内部事件生成 SHALL 通过 TurnJournal 统一记录（低优先级）

当前 durable events 由多个模块直接往 `Vec<StorageEvent>` 推送，导致“一个 step 产出了哪些事实、事件顺序为何如此”只能靠读细节拼出。系统 MUST 引入 `TurnJournal` 作为 turn 内部事件的统一收集器，提升可测试性和可解释性。

#### Scenario: TurnJournal 收集 turn 内部事件

- **WHEN** turn 执行期间产生 durable events
- **THEN** 系统 SHALL 通过 `TurnJournal` 统一收集
- **AND** `TurnJournal` SHALL 支持“给定 turn，输出全部按序事件”的查询语义
- **AND** SHALL NOT 改变现有事件持久化路径，仅替换 `Vec<StorageEvent>` 的直接使用

#### Scenario: TurnJournal 提升可测试性

- **WHEN** 单个 step 或 cycle 需要验证其产出的事件序列
- **THEN** 测试 SHALL 能够检查 `TurnJournal` 的内容
- **AND** SHALL NOT 需要从 `SessionState` 的全局存储中过滤事件来验证局部行为

### Requirement: display Phase SHALL 由事件投影驱动，SHALL NOT 被运行时代码直接变异

当前 `Phase` 存在两条写入路径：`submit.rs` 和 `execution.rs` 通过 `phase.lock()` 直接变异，`PhaseTracker` 通过事件类型推导。系统 MUST 消除直接变异路径，让 display `Phase` 完全由 `ProjectionRegistry` 中的 `PhaseTracker` 通过事件投影驱动。

#### Scenario: Phase 只由 ProjectionRegistry 驱动

- **WHEN** `TurnRuntimeStage` 从 `Preparing` 进入 `RunningModel`
- **THEN** 系统 SHALL NOT 直接 `phase.lock() = Phase::Thinking`
- **AND** SHALL 通过持久化一条触发 phase 变更的事件（如 `UserMessage`），让 `PhaseTracker` 推导出 `Phase::Thinking`
- **AND** `Phase::Streaming`（由 `AssistantDelta` / `AssistantFinal` 触发）和 `Phase::CallingTool`（由 `ToolCall` 触发）SHALL 继续由 `PhaseTracker` 事件推导

#### Scenario: recovery 后 Phase 由事件重放恢复

- **WHEN** session 从 checkpoint + tail events 恢复
- **THEN** display Phase SHALL 由 `PhaseTracker` 重放事件得到
- **AND** `normalize_recovered_phase()` SHALL 继续把 `Thinking / Streaming / CallingTool` 映射为 `Interrupted`
- **AND** runtime control state SHALL 不持有任何 Phase 信息（Phase 是 display-only）

### Requirement: interrupt 和 fork SHALL 通过 TurnRuntimeState transition API 完成，不绕过生命周期管控

当前 `interrupt_session()` 和 `fork_session()` 直接操作 `running`、`active_turn_id`、`cancel` 等散落字段，绕过任何 turn lifecycle 协调。系统 MUST 让 interrupt 和 fork 通过 `TurnRuntimeState` 的 typed transition API 操作，与正常提交共享同一套 control state 管控。它们不经过 `TurnCoordinator`（TurnCoordinator 是 per-turn 短暂对象，interrupt 发生时可能不存在活跃实例）。

#### Scenario: interrupt 通过 TurnRuntimeState::force_complete() 执行

- **WHEN** 用户请求中断正在运行的 session
- **THEN** 系统 SHALL 通过 `TurnRuntimeState::force_complete()` 触发中断
- **AND** `force_complete()` SHALL 原子递增 generation 并清理控制状态（与 Decision 19 的 generation counter 协同）
- **AND** SHALL NOT 直接操作 `cancel.lock()`、`active_turn_id.lock()` 或 `complete_session_execution()`

#### Scenario: fork 通过 TurnRuntimeState typed getter 读取 turn 状态

- **WHEN** 用户请求 fork 一个 session
- **THEN** 系统 SHALL 通过 `TurnRuntimeState` 的 typed getter 读取当前 turn 状态（stage、turn_id）
- **AND** SHALL NOT 直接读取 `phase.lock()` 或 `active_turn_id.lock()` 判断 turn 是否在运行

### Requirement: TurnRuntimeState 崩溃恢复 SHALL 不残留活跃 turn 控制状态

当前 `normalize_recovered_phase()` 把 display phase 从 `Thinking/Streaming/CallingTool` 降级为 `Interrupted`，但 runtime control state（active_turn_id、cancel、lease）没有相应的恢复逻辑。引入 `TurnRuntimeState` 后，系统 MUST 在恢复时将 runtime control state 重置为无活跃 turn，因为崩溃前的 turn 已不可恢复。

#### Scenario: recovery 时 TurnRuntimeState 重置为无活跃 turn

- **WHEN** session 从 checkpoint + tail events 恢复
- **THEN** `TurnRuntimeState` SHALL 初始化为无 active turn（`active_turn: None`，`running: false`）
- **AND** `running` 缓存镜像 SHALL 为 `false`
- **AND** 崩溃前未完成的 turn 的 display Phase SHALL 由 `normalize_recovered_phase()` 映射为 `Interrupted`

### Requirement: TurnCoordinator SHALL 使用 generation counter 防护 interrupt/resubmit 竞态

`interrupt_session()` 在清除控制状态后，被中断 turn 的异步 finalize 仍可能运行并覆盖新 turn 的控制状态。`TurnCoordinator` MUST 使用 generation counter 确保只有当前 generation 的 finalize 才能修改控制状态。

#### Scenario: stale finalize 不覆盖新 turn 控制状态

- **WHEN** Turn A 被中断后 Turn B 已开始执行
- **THEN** Turn A 的 finalize 调用 `complete()` 时 SHALL 检测 generation 不匹配
- **AND** SHALL 跳过控制状态清理（不清除 `running`、`active_turn_id`、`cancel`、`lease`）
- **AND** Turn B 的控制状态 SHALL 保持不变

#### Scenario: interrupt 无效化旧 generation

- **WHEN** `TurnRuntimeState::force_complete()` 被调用
- **THEN** SHALL 原子递增 generation 并清理控制状态
- **AND** 被中断 turn 的任何后续 finalize SHALL 因 generation 不匹配而被跳过

#### Scenario: 正常 complete 仅在 generation 匹配时执行

- **WHEN** turn 正常完成并调用 `complete(generation)`
- **THEN** 若 generation 与 `TurnRuntimeState` 当前 generation 匹配，SHALL 执行完整控制状态清理
- **AND** SHALL 原子返回 `Option<PendingManualCompactRequest>`

### Requirement: `session-runtime` SHALL 为重复的 turn/query helper 指定单一 canonical owner

`session-runtime` MUST 为 turn 终态投影、assistant summary 提取和 `session_id` 规范化等重复 helper 指定单一 canonical owner。其他子域调用方 SHALL 只复用该实现，SHALL NOT 继续在 `query/service`、`turn/submit`、`application` 或等价位置各自维护一份同类逻辑。

#### Scenario: turn outcome 与 terminal snapshot 复用同一投影逻辑
- **WHEN** 系统需要计算某个 turn 的 terminal snapshot 或 projected outcome
- **THEN** `query/service` 与其他消费方 SHALL 通过 `query/turn` 的 canonical helper 生成结果
- **AND** SHALL NOT 在多个调用点分别扫描事件并各自拼装相同语义

#### Scenario: assistant summary 提取不再多处实现
- **WHEN** finalize 路径或查询路径需要读取某个 turn 的 assistant summary
- **THEN** 系统 SHALL 通过同一份 summary 提取 helper 或 reducer 获取结果
- **AND** SHALL NOT 在 `turn/submit` 与 `query/turn` 中长期保留两套等价实现

#### Scenario: session id 规范化只有一个所有者
- **WHEN** 任意运行时入口需要把外部 `session_id` 输入转换为内部使用形式
- **THEN** 系统 SHALL 通过 `state::paths` 或等价 typed helper 完成规范化
- **AND** `application` 与多个 runtime 调用点 SHALL NOT 继续散落手写等价规范化逻辑

### Requirement: turn terminal projection SHALL 由同一 projector 同时服务增量、回放和重建路径

同一个 turn 的 terminal projection MUST 由一套共享 projector/reducer 逻辑生成。live append、query replay fallback 和 recovery rebuild SHALL 共用该逻辑，SHALL NOT 继续长期维护两套以上对 `TurnDone` / `Error` 的平行匹配分支。

#### Scenario: projection registry 与 query 共享同一 turn projector
- **WHEN** live append 更新某个 turn 的 terminal projection
- **THEN** `ProjectionRegistry` SHALL 通过共享 turn projector/reducer 更新结果
- **AND** query fallback SHALL 复用同一 projector 逻辑

#### Scenario: rebuild 与 live append 产出一致 terminal projection
- **WHEN** 系统分别通过 recovery rebuild 和 live append 处理等价的 turn 事件序列
- **THEN** 它们 SHALL 产出相同的 `TurnProjectionSnapshot`
- **AND** SHALL NOT 因为走不同入口而出现 terminal kind / last error 漂移

### Requirement: post-compact durable events SHALL 由共享 builder 生成

主动 compact、reactive compact 和 manual compact 之后写入的 durable 事件序列 MUST 由共享 builder 生成。该 builder SHALL 统一负责 `compact_applied`、recent user context digest/messages 和 file recovery messages 的构造；各调用方只负责提供 trigger、上下文与 compact result。

#### Scenario: 不同 compact 路径复用同一事件 builder
- **WHEN** proactive、reactive 或 manual compact 成功完成
- **THEN** 系统 SHALL 通过同一共享 builder 生成后续 durable 事件序列
- **AND** SHALL NOT 在三个调用点长期维护三套等价的事件组装逻辑

#### Scenario: compact 事件序列在不同 trigger 下结构保持一致
- **WHEN** 仅 compact trigger 不同，但 compact result 结构等价
- **THEN** 生成的 post-compact durable 事件结构 SHALL 保持一致
- **AND** 不同路径的差异 SHALL 仅来自 trigger 和对应上下文值，而不是事件拼装规则分叉

### Requirement: `session-runtime` crate 根导出面 SHALL 收口到稳定 façade 与稳定事实

`session-runtime` crate 根的公开导出 MUST 只保留稳定 façade、稳定 snapshot/result 和确实面向外层合同的 read-model facts。低层 orchestration helper、路径规范化函数和仅用于 runtime 内部拼装的辅助类型 SHALL NOT 继续作为 crate 根默认导出面。

#### Scenario: orchestration helper 不再从 crate 根外泄
- **WHEN** 外层 crate 依赖 `session-runtime`
- **THEN** 它们 SHALL 通过 `SessionRuntime` 的公开方法或 port blanket impl 消费运行时能力
- **AND** SHALL NOT 依赖 crate 根暴露的低层 helper、执行辅助或路径规范化工具完成编排

#### Scenario: 稳定 read-model facts 仍可继续暴露
- **WHEN** 某个类型已经作为 terminal / conversation 的稳定 authoritative facts 被上层 surface 消费
- **THEN** `session-runtime` MAY 继续公开该类型
- **AND** 本次收口 SHALL 聚焦 orchestration helper 与内部运行时辅助，不把 terminal read-model 的后续隔离强行并入同一阶段

### Requirement: `session-runtime` 拥有会话 durable projection 算法与快照

凡是依赖 session event 流恢复、服务于单 session authoritative read model 的 projection 算法与快照类型，`session-runtime` SHALL 作为唯一 owner。

这至少包括：

- input queue replay / projection 算法
- 其他需要根据 durable 事件重建的单 session 派生事实

#### Scenario: input queue replay is owned by session-runtime

- **WHEN** 检查 input queue 从 durable 事件恢复队列状态的实现
- **THEN** 该 replay / projection 算法 SHALL 位于 `session-runtime`
- **AND** `core` 不再保留等价的会话投影实现

#### Scenario: turn projection snapshot belongs to session-runtime

- **WHEN** 某个查询或恢复路径需要读取 turn projection 结果
- **THEN** projector、query、watcher 与等价的业务语义 SHALL 位于 `session-runtime`
- **AND** 若某个共享 checkpoint 载体暂时定义在 `core`，它也只作为跨 crate 合同存在，不改变 `session-runtime` 的业务 owner 地位

---

### Requirement: `session-runtime` 通过稳定端口消费副作用能力

当会话执行路径需要 durable tool result persist、项目目录解析或其他环境副作用时，`session-runtime` SHALL 通过稳定端口消费 adapter 提供的能力，而不是依赖 `core` 中的具体实现 helper。

#### Scenario: session-runtime does not reach into core for side effects

- **WHEN** 检查 `session-runtime` 中需要文件系统或 durable persist 的路径
- **THEN** 它们 SHALL 通过 port trait 调用外部能力
- **AND** 不再依赖 `core` 内的具体 IO / shell helper
