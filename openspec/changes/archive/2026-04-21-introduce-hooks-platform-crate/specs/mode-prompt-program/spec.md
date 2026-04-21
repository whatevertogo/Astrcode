## ADDED Requirements

### Requirement: mode and builtin prompt behavior MAY be delivered through lifecycle hook effects while preserving PromptDeclaration injection

mode 专属或 builtin 的运行时 prompt 行为 MAY 由 hooks 平台的 turn-level effects 产出，但最终 MUST 仍以 `PromptDeclaration` 进入既有 prompt 组装管线。系统 SHALL 不为 hooks-generated prompt 另开平行渲染旁路。

#### Scenario: builtin plan overlay is emitted by a before-turn hook

- **WHEN** builtin `plan` mode 需要根据当前 session / artifact / workflow 状态追加动态 prompt
- **THEN** 系统 MAY 通过 builtin `BeforeTurnSubmit` hook 产出对应 declarations
- **AND** 这些 declarations SHALL 与 mode prompt program 的其他输出一起走标准 `PromptDeclaration` 注入路径

#### Scenario: hook-generated prompt remains visible to PromptDeclarationContributor

- **WHEN** adapter-prompt 渲染 prompt declarations
- **THEN** 它 SHALL 能以与其他 declarations 相同的方式渲染 hooks-generated prompt
- **AND** SHALL 不需要识别一条新的 hooks 专用渲染旁路
