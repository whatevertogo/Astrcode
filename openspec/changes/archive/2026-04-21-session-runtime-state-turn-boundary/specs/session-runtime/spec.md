## MODIFIED Requirements

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

#### Scenario: SessionState 不再直接拥有 runtime control state

- **WHEN** 检查 `SessionState` 结构
- **THEN** 其字段只包含 projection registry、writer、broadcaster 与等价的 durable/projection 基础设施
- **AND** `TurnRuntimeState` SHALL 由 `turn` 子域定义并由单 session live truth owner 单独持有

#### Scenario: reload 后 display phase 仍从 durable 事件恢复

- **WHEN** 一个 session 从 durable 历史冷恢复
- **THEN** 系统 SHALL 从事件投影恢复 display phase
- **AND** SHALL NOT 依赖进程内残留的 runtime control state 判断该 session 的最终展示状态

#### Scenario: prepare / complete / interrupt 只维护 runtime control，不直接写 display Phase

- **WHEN** `TurnRuntimeState::prepare()`、`complete()` 或 `interrupt_if_running()` 被调用
- **THEN** 系统 SHALL 只更新 active turn、generation、cancel、compacting 与 running 等 runtime control 字段
- **AND** display `Phase` SHALL 继续只由 durable events 经 `PhaseTracker` 投影得到
- **AND** SHALL NOT 在这些 runtime control transition 中直接 `phase.lock()` 或等价方式同步设置 display Phase

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
