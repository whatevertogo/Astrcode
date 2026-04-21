## MODIFIED Requirements

### Requirement: `actor`、`observe`、`query` 必须按推进、订阅、拉取三类语义分离

`session-runtime` SHALL 固定以下语义边界：

- `actor` 只负责推进与持有单 session live truth
- `observe` 只负责推送/订阅语义、scope/filter、replay/live receiver 与状态源整合
- `query` 只负责拉取、快照与投影

`query` MAY 读取 durable event 与 projected state，但 MUST NOT 负责推进、副作用或长时间持有运行态协调逻辑。

conversation/tool display 的 authoritative read model MUST 归属于 `query` 子域；它可以聚合工具调用、流式输出、终态字段与 child 关联等单 session 读取语义，但 MUST NOT 把 HTTP/SSE framing、客户端补丁策略或 surface 样式逻辑带入 `session-runtime`。

turn terminal 等待、running turn watcher 与等价的运行态等待逻辑 SHALL NOT 归属于 `query`；它们 MUST 归属于 `turn` 子域。

#### Scenario: actor 不再承载观察视图拼装

- **WHEN** 检查 `actor` 子域实现
- **THEN** 其中只包含 session 推进、actor 生命周期与 live truth 管理
- **AND** 不包含 observe 快照投影或外部订阅协议映射

#### Scenario: query 只返回读取结果

- **WHEN** `application` 或 `server` 通过 `SessionRuntime` 发起读取
- **THEN** `query` 子域只返回 snapshot、projection 或 query result
- **AND** 不会因为查询路径隐式追加 durable 事件或推进 turn

#### Scenario: conversation 工具展示聚合落在 query 子域

- **WHEN** 上层需要读取某个 session 的工具展示结构、conversation hydration 或 catch-up 结果
- **THEN** `query` 子域 SHALL 直接返回 authoritative conversation/tool display facts
- **AND** 上层 MUST NOT 重新从原始 transcript record 或 replay/live receiver 组装同类语义

#### Scenario: observe 不承载 UI 级工具聚合语义

- **WHEN** `observe` 暴露 replay/live receiver 或相关订阅结果
- **THEN** 它 SHALL 只表达订阅与恢复语义
- **AND** MUST NOT 成为 tool block、conversation block 或等价 UI 读模型的长期所有者

#### Scenario: query 不再拥有 turn terminal 等待循环

- **WHEN** 检查 `query/service.rs` 或等价 query façade
- **THEN** 其中不再包含 `wait_for_turn_terminal_snapshot()` 这类基于 broadcaster 的等待循环
- **AND** turn terminal 等待 SHALL 归属 `turn/watcher.rs` 或等价的 turn-owned 模块

### Requirement: `state` 子域 SHALL 只持有 grouped runtime state 与 projection reducers

`session-runtime/state` 子域 MUST 只负责 durable projection state、projection reducer、durable cache、writer/broadcast 基础设施与相关 typed getter/setter。它 SHALL NOT 承担 turn runtime lifecycle control、workflow 编排、phase 业务语义解释或上层 use-case 判断。

`TurnRuntimeState`、`CompactRuntimeState`、`ActiveTurnState`、`ForcedTurnCompletion`、`PendingManualCompactRequest` 等运行时控制类型 MUST 归属于 `turn` 子域，而不是 `state`。

#### Scenario: state 子域只保留 durable/projection 真相

- **WHEN** `state` 子域维护 phase、mode、turn projection、child sessions、tasks 与 input queue
- **THEN** 这些状态 SHALL 继续以 projection reducer 或 durable cache 的形式存在
- **AND** `state` SHALL NOT 再持有 active turn、cancel token、turn lease 或 compact runtime control

#### Scenario: state 子域不解释 workflow business signal

- **WHEN** 上层 workflow 需要解释 approval、replan 或 phase bridge 信号
- **THEN** `state` 子域 SHALL 只提供必要的 authoritative facts
- **AND** SHALL NOT 在该子域内部持有 workflow-specific 分支逻辑

#### Scenario: SessionState 不再提供 turn runtime proxy

- **WHEN** 检查 `SessionState` 的公开方法
- **THEN** 不再存在 `prepare_execution()`、`complete_execution_state()`、`interrupt_execution_if_running()`、`cancel_active_turn()`、`is_running()`、`active_turn_id_snapshot()`、`manual_compact_pending()`、`compacting()`、`set_compacting()`、`request_manual_compact()` 等 turn runtime proxy
- **AND** turn 路径 SHALL 直接通过 turn-owned runtime handle 推进控制状态

#### Scenario: 外部 crate 不再通过 SessionState proxy 搭建测试场景

- **WHEN** `application` 或 `server` 的测试需要构造 running turn、completed turn 或 deferred compact 场景
- **THEN** 它们 SHALL 通过 `SessionRuntime` 稳定 façade、调用方本地 test support 或语义化 helper 搭建
- **AND** SHALL NOT 继续依赖 `SessionState` runtime proxy

### Requirement: `turn` 子域 SHALL 通过显式 transition API 推进 runtime lifecycle

`session-runtime/turn` 子域推进一次 turn 时 MUST 调用显式的 runtime lifecycle transition API，而不是在多个入口直接写底层状态字段。`submit`、`finalize`、`interrupt`、deferred compact 与 turn terminal watcher 相关路径 SHALL 共享同一组 turn-owned runtime lifecycle 语义。

#### Scenario: submit 与 finalize 共享统一 transition 入口

- **WHEN** turn 从待执行进入运行中，或从运行中进入终止状态
- **THEN** `submit` 与 `finalize` 路径 SHALL 通过同一组 transition API 更新 runtime lifecycle
- **AND** SHALL NOT 分别直接修改 `active_turn_id`、`lease`、`cancel` 或等价字段

#### Scenario: interrupt 路径复用同一 lifecycle 模型

- **WHEN** 当前 turn 被中断
- **THEN** `interrupt` 路径 SHALL 使用同一 runtime lifecycle 模型把 turn 标记为中断并清理控制状态
- **AND** SHALL NOT 通过单独的旁路状态重置逻辑绕过统一 transition 约束

#### Scenario: watcher 归 turn 子域所有

- **WHEN** 上层需要等待某个 turn 到达可判定终态
- **THEN** 系统 SHALL 通过 `turn` 子域提供的 watcher 能力完成
- **AND** watcher MAY 订阅 broadcaster 并在 lagged / closed 时回放恢复
- **AND** 这类等待语义 SHALL NOT 继续放在 `query` 子域
