## ADDED Requirements

### Requirement: SessionState 持有当前协作模式
系统 SHALL 在 `SessionState` 中新增 `session_mode: StdMutex<CollaborationMode>` 字段，默认值为 `Execute`。

#### Scenario: 新会话默认 Execute 模式
- **WHEN** SessionState::new() 被调用
- **THEN** session_mode 值为 CollaborationMode::Execute

#### Scenario: 读取当前模式
- **WHEN** 调用 session_state.current_mode()
- **THEN** 返回当前 session_mode 的值

### Requirement: 模式切换通过 StorageEvent 持久化
系统 SHALL 在 `StorageEventPayload` 中新增 `ModeChanged` 变体，包含：
- `from`: 切换前的 CollaborationMode
- `to`: 切换后的 CollaborationMode
- `source`: ModeTransitionSource（Tool / User | UI）
- `timestamp`

#### Scenario: 模式切换产生事件
- **WHEN** session_mode 从 Execute 切换到 Plan
- **THEN** 一条 `ModeChanged { from: Execute, to: Plan, source: Tool }` 事件被写入 storage

#### Scenario: 旧会话 replay 不受影响
- **WHEN** replay 不包含 ModeChanged 事件的旧会话
- **THEN** session_mode 保持默认值 Execute，不报错

### Requirement: 模式切换是原子操作
系统 SHALL 保证模式切换过程中，session_mode 的更新和事件的持久化在同一个锁范围内完成。

#### Scenario: 并发切换请求的串行化
- **WHEN** 两个并发的 switchMode 请求同时到达
- **THEN** 第二个请求 MUST 等待第一个完成后再执行，不会出现中间状态
