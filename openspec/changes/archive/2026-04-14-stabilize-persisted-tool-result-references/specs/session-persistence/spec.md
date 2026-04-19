## ADDED Requirements

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
