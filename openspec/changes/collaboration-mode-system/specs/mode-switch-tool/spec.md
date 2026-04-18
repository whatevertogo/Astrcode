## ADDED Requirements

### Requirement: switchMode builtin tool
系统 SHALL 提供 `switchMode` builtin tool，允许 LLM 在 step 中请求切换模式。工具参数：
- `mode`: 目标模式名称（字符串）
- `reason`: 切换原因（可选）

#### Scenario: LLM 成功切换到允许的模式
- **WHEN** LLM 调用 switchMode("plan", "任务复杂，需要先做方案")
- **AND** plan 模式的 entry_policy 为 LlmCanEnter
- **THEN** tool 返回成功，内容为"模式已切换到 plan，下一 turn 将使用新工具集"

#### Scenario: LLM 请求切换到 UserOnly 模式被拒绝
- **WHEN** LLM 调用 switchMode("execute", "准备执行")
- **AND** execute 模式从 plan 切换的 transition requires_confirmation=true
- **THEN** tool 返回错误"此切换需要用户确认，请提示用户使用 /mode execute"

#### Scenario: LLM 请求切换到不存在的模式
- **WHEN** LLM 调用 switchMode("nonexistent", ...)
- **THEN** tool 返回错误"未知模式: nonexistent"

#### Scenario: switchMode 产生 StorageEvent
- **WHEN** switchMode 成功执行
- **THEN** 一条 ModeChanged 事件被持久化，source 为 Tool

### Requirement: switchMode 不改变当前 step 的工具
系统 SHALL 保证 switchMode 在当前 step 内不改变工具集。工具切换在下一个 turn 开始时生效。

#### Scenario: Plan 模式下 switchMode("execute") 后当前 step 工具不变
- **WHEN** LLM 在 Plan 模式下的 step 3 调用 switchMode("execute")
- **THEN** step 3 后续的工具调用仍然只使用 Plan 模式的工具
- **AND** session_mode 已更新为 Execute
- **AND** 下一个 turn 开始时编译出 Execute 模式的完整工具集
