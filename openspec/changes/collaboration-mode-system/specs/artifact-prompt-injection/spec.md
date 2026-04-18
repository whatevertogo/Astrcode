## ADDED Requirements

### Requirement: Execute 模式自动引用 accepted plan artifact
系统 SHALL 在 Execute 模式的 prompt 编译阶段，自动查找 active_artifacts 中 status=Accepted 且 kind="plan" 的 artifact，并将其 Full 级渲染注入 prompt。

#### Scenario: 存在 accepted plan 时注入
- **WHEN** 当前模式为 Execute
- **AND** active_artifacts 包含 kind="plan", status=Accepted 的 artifact
- **THEN** prompt 中包含该 plan 的完整步骤、假设和风险说明

#### Scenario: 不存在 accepted plan 时不注入
- **WHEN** 当前模式为 Execute
- **AND** active_artifacts 中没有 accepted 的 plan artifact
- **THEN** prompt 中不包含 plan 引用块

#### Scenario: 多个 accepted plan 只注入最新的
- **WHEN** active_artifacts 包含多个 accepted plan artifact
- **THEN** 只注入 artifact_id 最大的（最新创建的）那个

### Requirement: Plan artifact 注入为 Dynamic 层 PromptDeclaration
系统 SHALL 将 plan artifact 渲染为 `PromptDeclaration`，属性为：
- `block_id`: "mode.artifact.plan"
- `layer`: SystemPromptLayer::Dynamic
- `kind`: PromptDeclarationKind::ExtensionInstruction
- `always_include`: true

#### Scenario: 渲染内容包含步骤列表
- **WHEN** plan artifact 的 PlanContent 有 3 个步骤
- **THEN** 注入的 PromptDeclaration content 包含这 3 个步骤的描述
