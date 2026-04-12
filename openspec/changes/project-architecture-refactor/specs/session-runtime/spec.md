## ADDED Requirements

### Requirement: `session-runtime` 作为唯一会话真相面

`session-runtime` crate SHALL 作为唯一会话真相面，负责：

- session 目录
- session actor
- session state
- turn loop
- interrupt / replay / branch
- observe / mailbox / routing
- durable event append
- session catalog 广播

#### Scenario: session-runtime 持有 session registry

- **WHEN** 检查 `SessionRuntime` 的核心结构
- **THEN** 它持有 session registry（例如 `DashMap<SessionId, Arc<SessionActor>>`）

#### Scenario: kernel 不再持有 session registry

- **WHEN** 检查 `kernel` crate
- **THEN** 不存在 session actor 目录或 session state 真相容器

---

### Requirement: `session-runtime` 隐藏内部并发容器

`session-runtime` MAY 在内部使用 `DashMap`、`RwLock`、`Mutex`，但 SHALL NOT 将这些并发容器直接暴露给外部。

#### Scenario: 外部拿到的是 handle 或 snapshot

- **WHEN** `application` 或 `server` 查询 session 状态
- **THEN** 获取的是 typed handle、snapshot 或查询结果
- **AND** 不是内部 `DashMap` 或锁保护对象

---

### Requirement: session 标识使用 newtype

`session-runtime` 内部和对外接口 SHALL 使用 `SessionId`、`TurnId`、`AgentId` 这类强类型标识，而不是以裸 `String` 作为核心 API 主类型。

#### Scenario: public API 不以 String 代表 session 身份

- **WHEN** 检查 `SessionRuntime` 的公共方法
- **THEN** 会话与 turn 身份通过强类型 ID 传递

---

### Requirement: `runtime-session` 迁入 `session-runtime/state`

`runtime-session` 的 `SessionState` 与相关状态逻辑 SHALL 迁入 `session-runtime/state`。

#### Scenario: runtime-session 最终删除

- **WHEN** 清理阶段完成
- **THEN** workspace 中不再包含 `runtime-session`

---

### Requirement: `runtime-agent-loop` 迁入 `session-runtime/turn`

`runtime-agent-loop` SHALL 迁入 `session-runtime/turn`，成为会话执行面的核心实现。

#### Scenario: turn loop 由 session-runtime 驱动

- **WHEN** application 请求执行一个 turn
- **THEN** 由 `SessionRuntime::run_turn(...)` 或等价入口驱动完整 turn loop

---

### Requirement: `runtime-execution` 迁入 `session-runtime/actor` 与 `context`

`runtime-execution` SHALL 迁入 `session-runtime/actor` 与 `session-runtime/context`，不再保留独立 crate。

#### Scenario: 子 agent 执行编排属于 session-runtime

- **WHEN** 检查根 agent / 子 agent 的执行编排代码
- **THEN** 它们位于 `session-runtime`

---

### Requirement: `runtime/service/session` 迁入 `session-runtime/catalog`

当前 runtime 中的 session 创建、加载、删除、列表、目录广播等逻辑 SHALL 统一归入 `session-runtime/catalog`。

#### Scenario: session 列表由 session-runtime 返回

- **WHEN** `application` 需要列出当前所有 session
- **THEN** 通过 `session-runtime` 获取

#### Scenario: session catalog 广播由 session-runtime 持有

- **WHEN** 新建、删除、分叉 session
- **THEN** `session-runtime` 发出 session catalog 级广播

---

### Requirement: durable append 是 session-runtime 主路径职责

`session-runtime` SHALL 持有 `Arc<dyn EventStore>`，并把 durable append 作为主路径职责，而不是异步订阅副作用。

#### Scenario: 事件先持久化再继续推进执行

- **WHEN** SessionActor 追加关键事件
- **THEN** 通过 `EventStore::append()` 显式落盘
- **AND** durability 是执行主路径的一部分

---

### Requirement: SessionActor 不直接持有 provider

SessionActor SHALL 只通过 `kernel` 间接调用 tool / llm / prompt / resource provider。

#### Scenario: SessionActor 字段干净

- **WHEN** 检查 `SessionActor` 字段
- **THEN** 不存在 `LlmProvider`、`PromptProvider`、`ToolProvider`、`ResourceProvider` 的直接字段

---

### Requirement: 协作工具桥接实现迁入 session-runtime

`SubAgentExecutor` 和 `CollaborationExecutor` 的实际执行桥接 SHALL 位于 `session-runtime`，因为它们本质上操作 session / agent 真相。

#### Scenario: runtime 不再承载桥接实现

- **WHEN** 清理阶段完成
- **THEN** 旧 `runtime` crate 中不再保留这两个桥接实现
