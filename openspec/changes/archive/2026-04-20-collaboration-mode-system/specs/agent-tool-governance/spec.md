## ADDED Requirements

### Requirement: collaboration guidance SHALL be generated from the current governance mode

当当前 session 可使用协作工具时，系统渲染给模型的协作 guidance MUST 来自当前 governance mode 编译得到的 action policy 与 prompt program，而不是固定的全局静态文本。

#### Scenario: execute mode renders the default collaboration protocol

- **WHEN** 当前 session 处于 builtin `execute` mode
- **THEN** 系统 SHALL 继续渲染默认的四工具协作协议
- **AND** 其行为语义 SHALL 与当前默认 guidance 保持等价

#### Scenario: restricted mode hides forbidden collaboration actions

- **WHEN** 当前 governance mode 禁止某类协作动作，例如新的 child delegation
- **THEN** 系统 SHALL 不向模型渲染鼓励该动作的 guidance
- **AND** SHALL 只保留当前 mode 允许的协作决策协议

### Requirement: collaboration guidance SHALL reflect mode-specific delegation constraints

协作 guidance MUST 体现当前 governance mode 对委派行为的额外约束，例如 child policy、reuse-first 限制与 capability mismatch 处置规则。

#### Scenario: mode narrows child reuse conditions

- **WHEN** 当前 mode 对 child reuse 设置了更严格的责任边界或能力前提
- **THEN** guidance SHALL 明确这些更严格的继续复用条件
- **AND** SHALL NOT 继续沿用更宽松的默认文案

#### Scenario: mode disables recursive delegation

- **WHEN** 当前 mode 的 child policy 禁止 child 再向下继续委派
- **THEN** guidance SHALL 明确当前分支的 delegation boundary
- **AND** SHALL NOT 鼓励模型继续 fan-out 新的 child 层级

### Requirement: CapabilityPromptContributor SHALL automatically reflect mode capability surface

`CapabilityPromptContributor` 通过 `PromptContext.tool_names` 和 `capability_specs` 渲染工具摘要和详细指南。mode 对工具面的约束 SHALL 自动反映在 contributor 的输出中，无需 contributor 自身感知 mode。

#### Scenario: mode removes collaboration tools from tool summary

- **WHEN** mode 编译的 capability router 移除了 spawn/send/close/observe 工具
- **THEN** `build_tool_summary_block` 的 "Agent Collaboration Tools" 分组 SHALL 为空
- **AND** 详细指南 SHALL 不包含被移除工具的条目

#### Scenario: mode restricts external tools

- **WHEN** mode 的 capability selector 排除了 source:mcp 或 source:plugin 工具
- **THEN** "External MCP / Plugin Tools" 分组 SHALL 仅包含未被排除的工具
- **AND** SHALL NOT 显示已被 mode 限制的工具

### Requirement: workflow_examples contributor SHALL delegate governance content to mode prompt program

`WorkflowExamplesContributor` 中与治理强相关的内容（协作协议、delegation modes、spawn 限制等）MUST 由 mode prompt program 生成的 PromptDeclarations 替代。contributor SHALL 仅保留非治理的 few-shot 教学内容。

#### Scenario: execute mode guidance is served from mode prompt program

- **WHEN** 当前 mode 为 `code`
- **THEN** 协作协议 guidance SHALL 来自 mode 编译的 PromptDeclarations
- **AND** `WorkflowExamplesContributor` 的 `child-collaboration-guidance` block SHALL 不再包含治理真相

#### Scenario: plan mode provides different collaboration guidance

- **WHEN** 当前 mode 为 `plan` 且允许有限 delegation
- **THEN** 协作 guidance SHALL 来自 plan mode 的 prompt program
- **AND** SHALL 包含 plan-specific 的委派策略说明
