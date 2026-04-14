## ADDED Requirements

### Requirement: turn loop SHALL 从输出截断处继续恢复

当 LLM 以 `max_tokens` 或等价输出上限原因结束当前 assistant 输出时，`session-runtime` 的 turn loop MUST 把该场景视为可恢复的 loop 分支，而不是只记录 warning 并直接结束本次输出。

#### Scenario: 无 tool call 的截断输出触发恢复

- **WHEN** 一轮 LLM 输出以输出上限结束，且当前 assistant 输出不包含 tool calls
- **THEN** 系统注入一条专用的 synthetic continuation prompt
- **AND** turn loop SHALL 在同一次 turn 内继续下一轮 LLM 调用

#### Scenario: 达到恢复上限后停止

- **WHEN** 输出截断恢复次数达到配置上限
- **THEN** turn loop SHALL 停止继续恢复
- **AND** 当前 turn 以明确 stop cause 结束

#### Scenario: 可恢复中的中间截断不立即变成最终失败

- **WHEN** 一次 `max_tokens` 截断仍满足自动恢复条件且尚未达到恢复上限
- **THEN** 系统 SHALL 注入 continuation prompt 并继续 turn loop
- **AND** 该中间截断 SHALL NOT 被当作最终失败立即释放

#### Scenario: 带 tool call 的截断输出不自动恢复

- **WHEN** assistant 输出以输出上限结束且包含 tool calls
- **THEN** 系统 SHALL NOT 自动注入 continuation prompt
- **AND** 该场景 SHALL 按更保守的结束或错误路径处理
