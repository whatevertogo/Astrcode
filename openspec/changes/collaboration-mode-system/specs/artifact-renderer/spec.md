## ADDED Requirements

### Requirement: ArtifactRenderer trait 定义渲染接口
系统 SHALL 定义 `ArtifactRenderer` trait，包含：
- `render(body: &ModeArtifactBody, level: RenderLevel) -> String`

`RenderLevel` 枚举包含：Summary、Compact、Full。

#### Scenario: Plan artifact Summary 级渲染
- **WHEN** PlanArtifactRenderer 以 RenderLevel::Summary 渲染一个 PlanContent
- **THEN** 输出为 1-2 句摘要文本

#### Scenario: Plan artifact Full 级渲染
- **WHEN** PlanArtifactRenderer 以 RenderLevel::Full 渲染一个 PlanContent
- **THEN** 输出包含完整步骤列表、假设、风险说明

### Requirement: 渲染结果可作为 PromptDeclaration 注入
系统 SHALL 将 ArtifactRenderer 的输出封装为 PromptDeclaration，注入到 consuming mode 的 prompt 中。

#### Scenario: Accepted plan artifact 注入到 Execute mode prompt
- **WHEN** 当前模式为 Execute
- **AND** active_artifacts 中存在一个 status=Accepted 的 plan artifact
- **THEN** plan artifact 的 Full 级渲染作为 PromptDeclaration 注入到 system prompt

#### Scenario: Compact 级渲染用于 auto compact 后
- **WHEN** 会话经历 auto compact
- **AND** active_artifacts 中存在 plan artifact
- **THEN** compact summary 使用 Summary 级渲染引用 plan artifact
