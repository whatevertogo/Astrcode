## ADDED Requirements

### Requirement: compile_mode_spec 编译模式执行规格
系统 SHALL 提供 `compile_mode_spec()` 函数，将 ModeSpec + 全部注册工具编译为 `ModeExecutionSpec`，包含：
- `visible_tools`: 当前模式可见的工具定义列表
- `mode_prompt`: 当前模式的约束 PromptDeclaration
- `mode_map_prompt`: 所有可用模式描述的 PromptDeclaration

#### Scenario: Plan 模式编译只读工具
- **WHEN** compile_mode_spec 以 ModeSpec("plan") 和包含 readFile + writeFile + shell 的工具注册表调用
- **THEN** visible_tools 仅包含 readFile，不包含 writeFile 和 shell

#### Scenario: Execute 模式编译全部工具
- **WHEN** compile_mode_spec 以 ModeSpec("execute") 调用
- **THEN** visible_tools 包含所有注册的工具

### Requirement: 工具编译使用授予白名单
系统 SHALL 通过 ToolGrantRule 白名单机制过滤工具。不在白名单中的工具不会出现在 visible_tools 中。

#### Scenario: Named 规则精确匹配
- **WHEN** tool_grants 包含 ToolGrantRule::Named("grep")
- **AND** 注册表中存在 grep 工具
- **THEN** grep 出现在 visible_tools 中

#### Scenario: Named 规则匹配不存在的工具
- **WHEN** tool_grants 包含 ToolGrantRule::Named("nonexistent")
- **AND** 注册表中不存在该工具
- **THEN** 编译不报错，该规则被忽略

#### Scenario: SideEffect 规则按类别过滤
- **WHEN** tool_grants 包含 ToolGrantRule::SideEffect(None)
- **AND** 注册表中 readFile 的 side_effect 为 None，writeFile 的 side_effect 为 Workspace
- **THEN** visible_tools 包含 readFile，不包含 writeFile

### Requirement: 编译注入当前模式约束 prompt
系统 SHALL 为当前模式生成一个 Dynamic 层的 PromptDeclaration，包含：
- `block_id`: "mode.current_constraint"
- `content`: ModeSpec.system_directive 的内容
- `layer`: SystemPromptLayer::Dynamic

#### Scenario: Plan 模式注入只读约束
- **WHEN** 当前模式为 Plan
- **THEN** 生成的 PromptDeclaration content 包含"只读分析"相关约束文本

### Requirement: 编译注入模式地图 prompt
系统 SHALL 生成一个 SemiStable 层的 PromptDeclaration，列出所有可用模式及其说明，包含：
- `block_id`: "mode.available_modes"
- `content`: 从 ModeCatalog.list_modes() 生成的模式描述
- `layer`: SystemPromptLayer::SemiStable

#### Scenario: 模式地图包含三个 builtin 模式
- **WHEN** ModeCatalog 包含 plan/execute/review 三个模式
- **THEN** 生成的 PromptDeclaration content 包含这三个模式的名称和描述

### Requirement: 编译结果集成到 TurnExecutionResources
系统 SHALL 在 turn 开始时调用 compile_mode_spec，将 visible_tools 替换 TurnExecutionResources 中的 tools 字段。

#### Scenario: Turn 开始时工具按模式编译
- **WHEN** 新 turn 以 session_mode=Plan 启动
- **THEN** TurnExecutionResources.tools 仅包含 Plan 模式授予的工具
