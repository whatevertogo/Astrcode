## ADDED Requirements

### Requirement: apply_mode_transition 统一模式切换入口
系统 SHALL 提供 `apply_mode_transition()` 函数作为所有模式切换的统一入口，参数包含：
- `session_state`: 目标会话状态
- `target_mode`: 目标 CollaborationMode
- `source`: ModeTransitionSource（Tool / User / UI）
- `translator`: EventTranslator

#### Scenario: 内部执行流程
- **WHEN** apply_mode_transition 被调用
- **THEN** 按序执行：验证转换合法性 → 检查 entry_policy → 更新 session_mode → 广播 ModeChanged 事件

### Requirement: 转换合法性验证
系统 SHALL 验证目标模式在当前模式的 ModeSpec.transitions 中是否合法。

#### Scenario: 合法转换通过
- **WHEN** 当前模式为 Plan，目标为 Execute
- **AND** ModeSpec("plan").transitions 包含 target_mode="execute"
- **THEN** 验证通过

#### Scenario: 非法转换被拒绝
- **WHEN** 当前模式为 Plan，目标为某个不在 transitions 列表中的模式
- **THEN** 返回错误"不允许从 plan 切换到 <target>"

### Requirement: entry_policy 检查
系统 SHALL 根据目标模式的 entry_policy 和 source 判断是否允许切换。

#### Scenario: LlmCanEnter + source=Tool 允许
- **WHEN** 目标模式 entry_policy 为 LlmCanEnter
- **AND** source 为 Tool（LLM 调用）
- **THEN** 允许切换

#### Scenario: UserOnly + source=Tool 拒绝
- **WHEN** 目标模式 entry_policy 为 UserOnly
- **AND** source 为 Tool（LLM 调用）
- **THEN** 拒绝切换，返回"需要用户手动切换"

#### Scenario: UserOnly + source=User 允许
- **WHEN** 目标模式 entry_policy 为 UserOnly
- **AND** source 为 User（/mode 命令或 UI）
- **THEN** 允许切换

### Requirement: transition requires_confirmation 检查
系统 SHALL 检查当前模式到目标模式的 transition 是否标记 requires_confirmation=true。如果是且 source=Tool，要求 LLM 提示用户确认。

#### Scenario: requires_confirmation=true + source=Tool 需要提示
- **WHEN** Plan → Execute 的 transition 标记 requires_confirmation=true
- **AND** source 为 Tool
- **THEN** 返回提示"此切换需要用户确认"

#### Scenario: requires_confirmation=true + source=User 直接通过
- **WHEN** Plan → Execute 的 transition 标记 requires_confirmation=true
- **AND** source 为 User
- **THEN** 直接通过，用户操作隐含确认
