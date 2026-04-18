## ADDED Requirements

### Requirement: Durable Session 真相由 `session-runtime` 持有

`session-runtime` SHALL 通过 `core::EventStore` 端口持久化并恢复 session 数据，替代旧的 `application` 内存 HashMap 真相。

#### Scenario: 创建 session 时建立 durable truth

- **WHEN** `create_session` 被调用
- **THEN** `session-runtime` 通过 `EventStore` 确保 session 已在持久化层创建
- **AND** live actor 与 durable event log 共享同一份 session 身份

#### Scenario: 加载未在内存中的 session

- **WHEN** `session_history`、`session_view` 或 `session_replay` 请求一个未加载的 session
- **THEN** `session-runtime` 从 `EventStore` 重放 durable 事件并重建 `SessionActor`

#### Scenario: 服务重启后恢复 session 列表

- **WHEN** server 重启后调用 `list_sessions`
- **THEN** `session-runtime` 通过 `EventStore` 枚举持久化的 session meta，而不是依赖启动期内存缓存

### Requirement: turn 事件持久化属于 `session-runtime` 主路径

`session-runtime` SHALL 在 turn 执行期间将每个关键 `StorageEvent` 追加到 `EventStore`，并把 durable append 视为执行主路径的一部分。

#### Scenario: turn 期间追加事件

- **WHEN** turn 执行期间产生 `StorageEvent`
- **THEN** 事件通过 `EventStore.append` 持久化
- **AND** 再广播到 session replay / SSE 消费者

#### Scenario: 持久化失败暴露为执行失败

- **WHEN** 关键事件追加失败
- **THEN** 系统记录错误
- **AND** 不静默吞掉 durable truth 断裂

### Requirement: `application` 不再持有 session shadow state

`application::App` SHALL 通过 `SessionRuntime` 访问 session 持久化能力，而不是再次缓存一份 session registry/history。

#### Scenario: App 委托 session 查询与删除

- **WHEN** `App` 处理 list/history/replay/delete 请求
- **THEN** 直接委托 `SessionRuntime`
- **AND** 不再维护 `HashMap<String, SessionEntry>` 一类的内存真相
