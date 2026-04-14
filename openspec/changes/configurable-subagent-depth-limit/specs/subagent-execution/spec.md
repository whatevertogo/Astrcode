## MODIFIED Requirements

### Requirement: subagent depth limit MUST be configurable and default to 3

runtime MUST 通过显式配置控制子代理最大嵌套深度；当用户未提供覆盖值时，默认值 MUST 为 `3`。

#### Scenario: no explicit runtime override

- **WHEN** 组合根在没有 `runtime.agent.max_subrun_depth` 覆盖的情况下启动 kernel
- **THEN** agent control MUST 使用默认最大嵌套深度 `3`

#### Scenario: explicit runtime override

- **WHEN** 配置文件提供 `runtime.agent.max_subrun_depth`
- **THEN** 组合根 MUST 将该值显式注入 kernel agent control limits
- **AND** 后续 spawn 校验 MUST 使用该覆盖值

### Requirement: prompt guidance MUST reflect the effective subagent depth limit

协作 prompt MUST 明确告诉模型当前生效的子代理最大嵌套深度，并指导其优先复用已有 child。

#### Scenario: prompt facts are resolved

- **WHEN** session-runtime 构建 prompt facts
- **THEN** metadata MUST 包含当前生效的 `agentMaxSubrunDepth`
- **AND** prompt vars MUST 暴露等价的 `agent.max_subrun_depth`

#### Scenario: collaboration guidance is rendered

- **WHEN** 当前 session 暴露 `spawn / send / observe / close` 四工具
- **THEN** prompt MUST 明确 `Idle` 是正常状态
- **AND** MUST 指导模型优先通过 `send(agentId, ...)` 复用已有 child
- **AND** MUST 明确命中 depth limit 后不要继续向更深层反复 spawn

### Requirement: spawn depth-limit failures MUST return actionable guidance

当 runtime 拒绝更深层 spawn 时，application MUST 返回可执行的用户/模型级建议，而不是内部错误。

#### Scenario: spawn exceeds configured depth

- **WHEN** child spawn 命中最大嵌套深度
- **THEN** application MUST 返回 `InvalidArgument`
- **AND** 错误消息 MUST 明确建议改用 `send / observe / close` 或在当前 agent 完成剩余工作
