## ADDED Requirements

### Requirement: governance prompt inputs SHALL resolve into the existing PromptPlan result model

mode prompt program、governance helper prompt、child contract prompt、skill-selected prompt 与其他治理级 prompt 输入 MUST 继续通过统一绑定路径汇入现有 `PromptPlan` 结果模型。系统 SHALL NOT 为治理侧单独引入平行的 prompt result IR。

#### Scenario: mode prompt declarations and governance helper prompts converge into PromptPlan

- **WHEN** 当前 turn 同时需要 mode prompt declarations、协作 guidance 与 child contract prompt
- **THEN** 系统 SHALL 先绑定这些治理 prompt 输入
- **AND** 由现有 prompt composer 产出单一 `PromptPlan`
- **AND** SHALL NOT 让其中任一路径绕过 `PromptPlan` 直接拼接最终 system prompt

#### Scenario: governance prompt binding preserves source metadata into prompt blocks

- **WHEN** 治理层注入一个由 mode contract 或 governance helper 生成的 prompt block
- **THEN** 该 block SHALL 能在结果模型中保留来源信息
- **AND** 调试或诊断时 SHALL 能区分它来自 mode prompt program、治理 helper、child contract 或 skill selection

### Requirement: mode prompt hooks SHALL extend governance prompt behavior without replacing the prompt pipeline

mode contract MAY 声明动态 prompt hooks，用于根据 artifact 状态、exit gate 状态或 workflow binding 调整 prompt 输入，但这些 hooks MUST 通过既有 `PromptDeclaration` / prompt composition 路径生效。

#### Scenario: mode prompt hook adds artifact-aware guidance

- **WHEN** 某个 mode 声明了与 artifact 状态相关的 prompt hook
- **THEN** 系统 SHALL 基于已绑定的 mode contract 产出额外 prompt input
- **AND** 这些输入 SHALL 通过现有 prompt declaration 与 prompt composer 路径渲染

#### Scenario: prompt hook cannot replace contributor internals

- **WHEN** 一个 mode prompt hook 试图改变 contributor 内部排序或渲染实现
- **THEN** 系统 SHALL 仅允许它追加或约束治理输入
- **AND** SHALL NOT 允许 mode hook 直接替换 `adapter-prompt` 的内部组装逻辑
