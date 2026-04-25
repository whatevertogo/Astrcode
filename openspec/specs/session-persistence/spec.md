## Requirements

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

---

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

---

### Requirement: `application` 不再持有 session shadow state

`application::App` SHALL 通过 `SessionRuntime` 访问 session 持久化能力，而不是再次缓存一份 session registry/history。

#### Scenario: App 委托 session 查询与删除

- **WHEN** `App` 处理 list/history/replay/delete 请求
- **THEN** 直接委托 `SessionRuntime`
- **AND** 不再维护 `HashMap<String, SessionEntry>` 一类的内存真相

---

### Requirement: tool-result replacement decisions SHALL 进入 durable event log

`session-runtime` MUST 将 persisted reference replacement decision 作为 durable 事件写入 `EventStore`，而不是仅保存在本轮内存状态中。

#### Scenario: fresh replacement 触发 durable event

- **WHEN** 某个 `tool_call_id` 首次被替换为 persisted reference
- **THEN** 系统 SHALL 追加一条 durable replacement 事件
- **AND** 该事件 SHALL 包含模型实际看到的 replacement 文本

#### Scenario: session 恢复后重建 replacement state

- **WHEN** 服务重启或按需加载一个未在内存中的 session
- **THEN** `session-runtime` SHALL 从 durable 事件重建 replacement state
- **AND** 后续 request assembly SHALL 继续重放与原会话一致的 replacement 文本

#### Scenario: replacement event 不替代原始 tool result 事实

- **WHEN** 某个 tool result 被 persisted reference replacement
- **THEN** 原始 `ToolResult` 事实 SHALL 仍然保留在 durable event log 中
- **AND** replacement 事件 SHALL 仅表达 prompt 消费层面的替换决策

---

### Requirement: session persistence SHALL 归属 `host-session`

事件日志、session meta、冷恢复、branch/fork durable truth 与 read model 恢复 MUST 归属 `host-session` crate，而不是继续由 live runtime 或已删除的 `application` 承担。

#### Scenario: 持久化 owner 与 live runtime 解耦
- **WHEN** 某个 turn 产生 durable events
- **THEN** `host-session` SHALL 负责其追加、恢复与后续投影
- **AND** `agent-runtime` SHALL 不再拥有全部持久化服务职责

---

### Requirement: session persistence SHALL 不依赖兼容影子状态

重构后系统 MUST 仅以事件日志与正式投影作为 session durable truth，SHALL NOT 通过旧 `application` 缓存、旧 runtime shadow state 或过渡兼容表维护第二套真相。

#### Scenario: 冷恢复不读取兼容缓存
- **WHEN** 服务重启后恢复某个 session
- **THEN** 系统 SHALL 仅从事件日志与正式投影恢复
- **AND** SHALL NOT 读取兼容 shadow state
