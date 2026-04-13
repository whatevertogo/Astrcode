## Requirements

### Requirement: `session-runtime` 是唯一会话真相面

`session-runtime` crate SHALL 统一负责会话生命周期与执行真相，包括：

- session 目录
- session actor / state
- session query / replay / history view
- turn loop
- interrupt / replay / branch
- observe / mailbox / routing
- durable event append
- session catalog 广播
- token budget 驱动的单 turn 自动续写
- turn 级 observability 汇总

#### Scenario: SessionRuntime 持有会话目录

- **WHEN** 检查 `SessionRuntime` 核心结构
- **THEN** 它持有 session registry（例如 `DashMap<SessionId, Arc<SessionActor>>`）

#### Scenario: SessionRuntime 提供会话查询与执行入口

- **WHEN** `server` 或 `application` 需要列 session、读取 history、replay 事件或提交 prompt
- **THEN** 统一通过 `SessionRuntime` 暴露的 query / mutation 入口完成
- **AND** 不再由 `application` 自己维护内存态 session history

#### Scenario: kernel 不再持有会话真相

- **WHEN** 检查 `kernel` crate
- **THEN** 不存在 session actor 目录或 session state 真相容器

---

### Requirement: 会话执行构造逻辑归 `session-runtime`

`build_agent_loop`、`LoopRuntimeDeps`、`AgentLoop`、`TurnRunner` SHALL 位于 `session-runtime/turn` 或 `session-runtime/factory`，不在 `kernel`。

#### Scenario: turn 由 session-runtime 完整驱动

- **WHEN** `application` 请求执行 turn
- **THEN** 通过 `SessionRuntime::run_turn(...)`（或等价入口）驱动完整执行

#### Scenario: turn 内预算与观测不泄漏到 application

- **WHEN** `application` 发起一次 prompt 提交
- **THEN** token budget、continue nudge、turn 级 observability 由 `session-runtime` 内部处理
- **AND** `application` 不拥有 turn 内循环策略

#### Scenario: kernel 不持有 turn 构造实现

- **WHEN** 检查 `kernel` 模块
- **THEN** 不存在 `build_agent_loop` 或 `LoopRuntimeDeps`

---

### Requirement: `runtime-session` / `runtime-agent-loop` / `runtime-execution` 迁入 `session-runtime`

以下旧层能力 SHALL 迁入 `session-runtime`：

- `runtime-session` -> `session-runtime/state`
- `runtime-agent-loop` -> `session-runtime/turn`
- `runtime-execution` -> `session-runtime/actor` 与 `session-runtime/context`
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
- **THEN** 返回 typed handle / snapshot / query result
- **AND** 不返回内部 `DashMap` 或锁对象

---

### Requirement: `session-runtime` 内部继续按单 session 职责分块

`session-runtime` 内部 SHALL 至少按以下职责分块组织，而不是把所有执行细节平铺在 crate 根：

- `state`
- `catalog`
- `actor`
- `turn`
- `context`
- `factory`
- `query`

#### Scenario: 单 session 真相与执行结构清晰

- **WHEN** 检查 `session-runtime/src`
- **THEN** 可以沿着 `state -> actor -> turn -> query` 的结构理解单 session 行为
- **AND** 不需要回到 `application` 中寻找会话真相
