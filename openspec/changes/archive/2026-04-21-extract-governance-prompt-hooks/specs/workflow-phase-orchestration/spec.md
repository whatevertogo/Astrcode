## ADDED Requirements

### Requirement: workflow phase prompt overlays SHALL resolve through workflow-scoped prompt hooks

workflow phase 的 prompt overlay MUST 通过 workflow-scoped governance prompt hook/provider 生成，而不是在 session 提交流程中按 phase id 写死 `if/else` 来构造 phase-specific declarations。

#### Scenario: executing phase receives bridge overlay from a workflow prompt hook

- **WHEN** active workflow 处于 `executing` phase，且 bridge state 已存在
- **THEN** 系统 SHALL 通过 `WorkflowPhaseOverlay` 类 governance prompt hook 生成 execute bridge declaration
- **AND** SHALL NOT 在主提交流程中直接调用 plan-specific helper 来拼接该 declaration

#### Scenario: planning phase uses its own overlay path without leaking executing guidance

- **WHEN** active workflow 处于 `planning` phase
- **THEN** 系统 SHALL 只解析匹配 planning phase 的 prompt hooks 或 mode-active hooks
- **AND** SHALL NOT 让 executing phase 的 bridge overlay 对模型可见

### Requirement: workflow prompt hooks SHALL consume resolved phase truth and SHALL NOT decide transitions

workflow prompt hooks MUST 建立在已解析的 workflow state、phase truth 与 bridge context 之上。它们可以把 phase truth 映射成 prompt overlay，但 SHALL NOT 解释自由文本、决定 signal 或触发 phase 迁移。

#### Scenario: transition is resolved before workflow overlay generation

- **WHEN** 用户输入触发 `planning -> executing` 的 workflow 迁移
- **THEN** 系统 SHALL 先完成 signal 解释、bridge 计算与 workflow state 持久化
- **AND** 再把新的 phase truth 作为 workflow prompt hook 输入，用于生成 executing overlay

#### Scenario: workflow prompt hook does not reinterpret free-text approvals

- **WHEN** workflow prompt hook 处理 `WorkflowPhaseOverlay` 输入
- **THEN** 它 SHALL 只消费已解析的 `phase_id`、artifact refs 与 bridge payload
- **AND** SHALL NOT 再根据用户自由文本自行判断是否属于 approval 或 replan 信号
