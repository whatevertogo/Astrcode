## Purpose

定义 agent-tool 协作的 prompt 治理规则，确保四工具（spawn / send / observe / close）在 prompt 层面遵循统一的决策协议，降低无效协作与过度 fan-out。

## Requirements

### Requirement: four-tool collaboration MUST follow a single decision protocol

当系统同时暴露 `spawn / send / observe / close` 四个协作工具时，prompt guidance MUST 把它们表达为一套互斥的下一步决策协议，而不是四个彼此独立的说明书。

#### Scenario: collaboration guidance is rendered

- **WHEN** 当前 session 可使用 `spawn / send / observe / close`
- **THEN** 系统 MUST 明确 `spawn` 用于新的、隔离的责任分支
- **AND** MUST 明确 `send` 用于同一 child 的下一条具体指令
- **AND** MUST 明确 `observe` 只在下一步决策依赖当前状态时使用
- **AND** MUST 明确 `close` 用于结束已经完成或不再需要的 child 分支

#### Scenario: child becomes idle after a completed turn

- **WHEN** child agent 完成一轮 turn 并回到 `Idle`
- **THEN** prompt guidance MUST 将 `Idle` 表达为正常可复用状态
- **AND** MUST NOT 把 `Idle` 暗示成需要立刻 respawn 的错误状态

### Requirement: spawn guidance MUST prefer reuse before fan-out

`spawn` 的 guidance MUST 把创建新 child 视为昂贵操作，并优先引导模型复用已有 child 或在当前 agent 内完成工作。

#### Scenario: idle child already owns the responsibility

- **WHEN** 当前存在一个处于 `Idle` 的 child，且其责任边界与待办工作一致
- **THEN** prompt guidance MUST 建议优先使用 `send`
- **AND** MUST NOT 将继续 `spawn` 新 child 作为默认推荐动作

#### Scenario: depth or fan-out limit is reached

- **WHEN** runtime 命中子代理深度或 sibling fan-out 限制
- **THEN** `spawn` 的 guidance MUST 明确建议改用 `send / observe / close`
- **AND** MUST 明确建议在当前 agent 内完成剩余工作，而不是继续重试新的 `spawn`

### Requirement: collaboration tool prompts MUST stay action-oriented and low-noise

四工具的 prompt metadata MUST 优先约束下一步动作，而不是重复解释底层 runtime 细节或鼓励无用思考。

#### Scenario: send prompt is rendered

- **WHEN** 系统向模型描述 `send`
- **THEN** prompt MUST 强调"一次发送一条具体的下一步指令"
- **AND** MUST 明确禁止把 `send` 用作状态探测或模糊催促

#### Scenario: observe prompt is rendered

- **WHEN** 系统向模型描述 `observe`
- **THEN** prompt MUST 明确它是同步非阻塞查询
- **AND** MUST 明确禁止在没有后续决策的情况下高频轮询

#### Scenario: close prompt is rendered

- **WHEN** 系统向模型描述 `close`
- **THEN** prompt MUST 明确它会级联关闭 child 子树
- **AND** MUST 明确它是"结束分支"的动作，而不是"探测状态"的动作
