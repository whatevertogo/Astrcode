## ADDED Requirements

### Requirement: `session-runtime` 是唯一会话真相面

`session-runtime` crate SHALL 统一负责会话生命周期与执行真相，包括：

- session 目录
- session actor / state
- turn loop
- interrupt / replay / branch
- observe / mailbox / routing
- durable event append
- session catalog 广播

#### Scenario: SessionRuntime 持有会话目录

- **WHEN** 检查 `SessionRuntime` 核心结构
- **THEN** 它持有 session registry（例如 `DashMap<SessionId, Arc<SessionActor>>`）

#### Scenario: kernel 不再持有会话真相

- **WHEN** 检查 `kernel` crate
- **THEN** 不存在 session actor 目录或 session state 真相容器

---

### Requirement: 会话执行构造逻辑归 `session-runtime`

`build_agent_loop`、`LoopRuntimeDeps`、`AgentLoop`、`TurnRunner` SHALL 位于 `session-runtime/turn` 或 `session-runtime/factory`，不在 `kernel`。

#### Scenario: turn 由 session-runtime 完整驱动

- **WHEN** `application` 请求执行 turn
- **THEN** 通过 `SessionRuntime::run_turn(...)`（或等价入口）驱动完整执行

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
