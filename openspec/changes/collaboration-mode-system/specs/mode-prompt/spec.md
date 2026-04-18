## ADDED Requirements

### Requirement: ModeMap prompt block 注入可用模式描述
系统 SHALL 在每 turn 的 prompt 中注入一个 "Available Modes" block（SemiStable 层），内容包含：
- 每个可用模式的名称和简短描述
- 每个模式的适用场景
- 哪些模式 LLM 可自行进入，哪些需要用户操作
- 切换方式说明

#### Scenario: ModeMap 包含三个 builtin 模式
- **WHEN** BuiltinModeCatalog 包含 plan/execute/review
- **THEN** ModeMap prompt block 包含这三个模式的名称、描述和进入策略

#### Scenario: LLM 能理解何时进入 Plan 模式
- **WHEN** ModeMap prompt block 被注入到 system prompt
- **THEN** 内容包含"当任务复杂、涉及多文件、或不确定最佳方案时"类似描述

### Requirement: CurrentMode prompt block 注入当前约束
系统 SHALL 在每 turn 的 prompt 中注入一个 "Current Mode" block（Dynamic 层），内容包含：
- 当前模式名称
- 当前模式的核心约束
- 如果有 active_artifacts，引用其 summary

#### Scenario: Plan 模式注入只读约束
- **WHEN** 当前模式为 Plan
- **THEN** CurrentMode block 内容包含"只使用只读工具"、"不修改文件"等约束

#### Scenario: Execute 模式引用已接受的 plan artifact
- **WHEN** 当前模式为 Execute
- **AND** active_artifacts 中存在一个 status=Accepted 的 plan artifact
- **THEN** CurrentMode block 引用该 plan 的 summary

#### Scenario: 模式切换后 prompt 自动更新
- **WHEN** session_mode 从 Plan 切换到 Execute
- **THEN** 下一个 step 的 prompt 中 CurrentMode block 内容更新为 Execute 的约束
