## ADDED Requirements

### Requirement: CollaborationMode 枚举定义协作阶段
系统 SHALL 定义 `CollaborationMode` 枚举，包含 `Plan`、`Execute`、`Review` 三个变体，作为跨 crate 稳定成立的语义词汇。

#### Scenario: 默认模式为 Execute
- **WHEN** 新会话创建时
- **THEN** `CollaborationMode` 默认值为 `Execute`

#### Scenario: 枚举可序列化为 camelCase
- **WHEN** `CollaborationMode::Plan` 被序列化为 JSON
- **THEN** 输出为 `"plan"`

### Requirement: ModeSpec 声明式定义模式规格
系统 SHALL 定义 `ModeSpec` 结构体，包含以下字段：
- `id`: 唯一标识（如 "plan"）
- `name`: 人类可读名称
- `description`: 模式说明（供 LLM 理解何时使用）
- `tool_grants`: `Vec<ToolGrantRule>` 工具授予规则
- `system_directive`: 模式约束提示词
- `entry_policy`: `ModeEntryPolicy` 进入策略
- `transitions`: `Vec<ModeTransition>` 合法转换规则
- `output_artifact_kind`: `Option<String>` 产出 artifact 的 kind

#### Scenario: ModeSpec 完整序列化
- **WHEN** 一个包含所有字段的 ModeSpec 被序列化
- **THEN** JSON 输出包含 toolGrants、systemDirective、entryPolicy、transitions、outputArtifactKind 等字段

#### Scenario: ModeSpec 缺失可选字段
- **WHEN** 一个没有 output_artifact_kind 的 ModeSpec 被序列化
- **THEN** JSON 中不包含 outputArtifactKind 字段

### Requirement: ToolGrantRule 定义工具授予策略
系统 SHALL 定义 `ToolGrantRule` 枚举，包含三个变体：
- `Named(String)`: 按工具名称精确匹配
- `SideEffect(SideEffect)`: 按 CapabilitySpec 的 side_effect 类别授予
- `All`: 授予全部工具

#### Scenario: SideEffect(None) 授予所有只读工具
- **WHEN** ToolGrantRule::SideEffect(None) 被用于编译工具列表
- **AND** 注册表中存在 readFile（side_effect=None）和 writeFile（side_effect=Workspace）
- **THEN** readFile 被授予，writeFile 被排除

#### Scenario: Named 授予指定工具
- **WHEN** ToolGrantRule::Named("readFile") 被用于编译工具列表
- **AND** 注册表中存在 readFile
- **THEN** readFile 被授予

### Requirement: ModeEntryPolicy 定义进入策略
系统 SHALL 定义 `ModeEntryPolicy` 枚举，包含三个变体：
- `LlmCanEnter`: LLM 可自行进入
- `UserOnly`: 仅用户可触发
- `LlmSuggestWithConfirmation`: LLM 可建议但需用户确认

#### Scenario: LlmCanEnter 模式下 LLM 调用 switchMode
- **WHEN** LLM 通过 switchMode tool 请求进入一个 entry_policy 为 LlmCanEnter 的模式
- **THEN** 切换被允许，无需用户确认

#### Scenario: UserOnly 模式下 LLM 调用 switchMode
- **WHEN** LLM 通过 switchMode tool 请求进入一个 entry_policy 为 UserOnly 的模式
- **THEN** 切换被拒绝，tool 返回错误信息"此模式需要用户手动切换"

### Requirement: ModeTransition 定义合法转换规则
系统 SHALL 定义 `ModeTransition` 结构体，包含：
- `target_mode`: 目标模式 ID
- `requires_confirmation`: 是否需要确认

#### Scenario: Plan → Execute 转换需要确认
- **WHEN** ModeSpec("plan") 的 transitions 包含 `{ target_mode: "execute", requires_confirmation: true }`
- **THEN** 从 plan 切换到 execute 需要用户确认

#### Scenario: Execute → Plan 转换不需要确认
- **WHEN** ModeSpec("execute") 的 transitions 包含 `{ target_mode: "plan", requires_confirmation: false }`
- **THEN** LLM 可直接从 execute 切换到 plan
