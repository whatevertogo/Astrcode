## Purpose

定义 governance mode 如何编译为 prompt program 生成 PromptDeclarations，以及 mode 如何控制 builtin prompt contributor 的行为。

## Requirements

### Requirement: mode SHALL compile to a prompt program that generates PromptDeclarations

每个 governance mode MUST 编译为一个 prompt program，该 program 在 turn 边界生成一组 `PromptDeclaration`，作为 `ResolvedTurnEnvelope` 的一部分注入 prompt 组装管线。

#### Scenario: execute mode compiles the default prompt program

- **WHEN** 当前 session 的 mode 为 builtin `code`
- **THEN** prompt program SHALL 生成与当前默认协作 guidance 等价的 PromptDeclarations
- **AND** 渲染结果 SHALL 与现有 `WorkflowExamplesContributor` 的 `child-collaboration-guidance` block 行为等价

#### Scenario: plan mode compiles a planning-oriented prompt program

- **WHEN** 当前 session 的 mode 为 builtin `plan`
- **THEN** prompt program SHALL 生成规划导向的 PromptDeclarations
- **AND** SHALL 包含规划方法论 guidance、输出格式约束、以及不允许直接执行的声明

#### Scenario: review mode compiles an observation-oriented prompt program

- **WHEN** 当前 session 的 mode 为 builtin `review`
- **THEN** prompt program SHALL 生成审查导向的 PromptDeclarations
- **AND** SHALL 不包含 spawn/send 协作协议 guidance

### Requirement: mode prompt program SHALL integrate through the existing PromptDeclaration injection path

mode 生成的 PromptDeclarations MUST 通过现有注入路径进入 prompt 组装，即 `TurnRunRequest.prompt_declarations` -> `TurnExecutionResources` -> `AssemblePromptRequest` -> `PromptOutputRequest.submission_prompt_declarations` -> `build_prompt_output()`，MUST NOT 开辟新的 prompt 注入旁路。

#### Scenario: mode declarations travel the standard path

- **WHEN** mode 编译生成 PromptDeclarations
- **THEN** 它们 SHALL 被放入 `AgentPromptSubmission.prompt_declarations`
- **AND** 通过 `submit_prompt_inner` -> `RunnerRequest` -> `TurnRunRequest` 标准路径进入 runner

#### Scenario: mode declarations are visible to PromptDeclarationContributor

- **WHEN** `PromptDeclarationContributor` (adapter-prompt) 渲染 prompt
- **THEN** 它 SHALL 能渲染 mode 生成的 declarations
- **AND** SHALL 对 mode declarations 和其他 declarations 使用相同的渲染逻辑

### Requirement: mode SHALL control which builtin prompt contributors are active

不同 mode 可以要求禁用或替换某些 builtin prompt contributor。mode spec SHALL 能声明对 contributor 的约束。

#### Scenario: execute mode keeps all contributors active

- **WHEN** 当前 mode 为 `code`
- **THEN** 所有现有 contributor（WorkflowExamplesContributor、AgentProfileSummaryContributor、CapabilityPromptContributor）SHALL 保持活跃
- **AND** 行为与当前默认等价

#### Scenario: mode disables AgentProfileSummaryContributor when delegation is forbidden

- **WHEN** 当前 mode 的 child policy 禁止创建 child 分支
- **THEN** `AgentProfileSummaryContributor` SHALL 不渲染（因为它只在 spawn 可用时激活）
- **AND** 这一行为 SHALL 自动发生（因为 mode 编译的 capability router 已移除 spawn 工具）

#### Scenario: mode replaces collaboration guidance content

- **WHEN** 当前 mode 要求不同的协作 guidance
- **THEN** `WorkflowExamplesContributor` 的治理专属内容 SHALL 被 mode prompt program 的 declarations 替代
- **AND** `WorkflowExamplesContributor` SHALL 仅保留非治理 few-shot 内容（如果有）

### Requirement: PromptFactsProvider SHALL resolve prompt facts against the mode-compiled envelope

`PromptFactsProvider.resolve_prompt_facts()` 构建的 `PromptFacts` MUST 与 mode 编译后的治理包络保持一致，包括 metadata 中的治理参数和 declarations 的可见性过滤。

#### Scenario: PromptFacts metadata reflects mode-resolved parameters

- **WHEN** mode 编译后的 envelope 指定了 max_spawn_per_turn = 0
- **THEN** `PromptFacts.metadata` 中的 `agentMaxSpawnPerTurn` SHALL 为 0
- **AND** SHALL NOT 使用 runtime config 中的原始值

#### Scenario: prompt declaration visibility aligns with mode capability surface

- **WHEN** mode 编译的 envelope 移除了某些工具
- **THEN** `prompt_declaration_is_visible` 过滤 SHALL 使用 envelope 的能力面
- **AND** 已被 mode 移除的工具对应的 declarations SHALL 不对模型可见

#### Scenario: profile context approvalMode reflects mode policy

- **WHEN** mode 的 action policies 包含审批要求
- **THEN** `build_profile_context` 中的 `approvalMode` SHALL 反映该模式
- **AND** SHALL 与 PolicyEngine 的实际行为一致

### Requirement: plugin mode SHALL be able to contribute custom prompt blocks without replacing contributors

插件 mode MUST 能通过 prompt program 注入自定义 prompt blocks，但 MUST NOT 直接替换、删除或修改现有 prompt contributor 的内部逻辑。

#### Scenario: plugin mode appends custom guidance

- **WHEN** 一个插件 mode 定义了自定义协作 guidance
- **THEN** 系统 SHALL 将该 guidance 编译为额外的 PromptDeclaration
- **AND** 现有 contributor 的渲染逻辑 SHALL 不受影响

#### Scenario: plugin mode cannot bypass contributor pipeline

- **WHEN** 一个插件 mode 尝试绕过 prompt 组装管线
- **THEN** 系统 SHALL 仅通过 PromptDeclaration 注入路径接受 mode 的 prompt 输入
- **AND** SHALL NOT 允许插件直接修改 prompt 组装中间产物
