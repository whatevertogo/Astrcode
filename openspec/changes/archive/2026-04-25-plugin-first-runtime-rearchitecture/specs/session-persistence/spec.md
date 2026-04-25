## ADDED Requirements

### Requirement: session persistence SHALL 归属 `host-session`

事件日志、session meta、冷恢复、branch/fork durable truth 与 read model 恢复 MUST 归属 `host-session` crate，而不是继续由 live runtime 或已删除的 `application` 承担。

#### Scenario: 持久化 owner 与 live runtime 解耦
- **WHEN** 某个 turn 产生 durable events
- **THEN** `host-session` SHALL 负责其追加、恢复与后续投影
- **AND** `agent-runtime` SHALL 不再拥有全部持久化服务职责

### Requirement: session persistence SHALL 不依赖兼容影子状态

重构后系统 MUST 仅以事件日志与正式投影作为 session durable truth，SHALL NOT 通过旧 `application` 缓存、旧 runtime shadow state 或过渡兼容表维护第二套真相。

#### Scenario: 冷恢复不读取兼容缓存
- **WHEN** 服务重启后恢复某个 session
- **THEN** 系统 SHALL 仅从事件日志与正式投影恢复
- **AND** SHALL NOT 读取兼容 shadow state
