## MODIFIED Requirements

### Requirement: four-tool collaboration MUST follow a single decision protocol

当系统同时暴露 `spawn / send / observe / close` 四个协作工具时，prompt guidance MUST 把它们表达为一套互斥的下一步决策协议，而不是四个彼此独立的说明书。`send` 必须按这次协作消息的方向表达不同语义，但仍保持同一个工具名；中间层 agent 可以同时具备 upward 和 downward 两种 `send` 能力。

#### Scenario: collaboration guidance is rendered

- **WHEN** 当前 session 可使用 `spawn / send / observe / close`
- **THEN** 系统 MUST 明确 `spawn` 用于新的、隔离的责任分支
- **AND** MUST 明确 `observe` 只在下一步决策依赖当前状态时使用
- **AND** MUST 明确 `close` 用于结束已经完成或不再需要的 child 分支

#### Scenario: downward send guidance is rendered

- **WHEN** 当前模型需要把下一步具体任务交给 direct child
- **THEN** guidance MUST 明确 `send` 用于向 direct child 发送下一条具体指令
- **AND** MUST 明确要求使用稳定 `agentId`
- **AND** MUST 明确禁止把它用于状态探测或模糊催促

#### Scenario: upward send guidance is rendered

- **WHEN** 当前模型需要把当前分支结果回给 direct parent
- **AND** 当前 agent 存在 direct parent
- **THEN** guidance MUST 明确 `send` 用于向 direct parent 汇报 `progress`、`completed`、`failed` 或 `close_request`
- **AND** MUST 明确禁止跨树闲聊、越级发送或把上行消息伪装成普通 summary

#### Scenario: middle agent can use both send directions in one turn

- **WHEN** 当前 agent 既有 direct parent，也拥有一个或多个 direct child
- **THEN** guidance MUST 明确同一个 agent 在同一轮里既可能向上 `send` 汇报，也可能向下 `send` 委派
- **AND** MUST 明确这两种调用只在参数分支、ownership 校验与 routing 目标上不同，而不是两个不同工具

#### Scenario: child becomes idle after a completed turn

- **WHEN** child agent 完成一轮 turn 并回到 `Idle`
- **THEN** prompt guidance MUST 将 `Idle` 表达为正常可复用状态
- **AND** MUST NOT 把 `Idle` 暗示成需要立刻 respawn 的错误状态

### Requirement: collaboration tool prompts MUST stay action-oriented and low-noise

协作工具的 prompt metadata MUST 优先约束下一步动作，而不是重复解释底层 runtime 细节或鼓励无用思考。unified `send` 的双向语义必须被清楚说明，但不能把整套 delivery 实现细节塞进 description。

#### Scenario: send prompt is rendered

- **WHEN** 系统向模型描述 `send`
- **THEN** prompt MUST 明确它是统一协作消息入口
- **AND** MUST 说明 upstream 与 downstream 会使用不同参数分支
- **AND** MUST 明确 `send` 不是自由双向聊天接口

#### Scenario: progress does not expand first-wave UI scope

- **WHEN** 首批 unified `send` 已支持 `progress`
- **THEN** guidance MUST 允许 runtime 持久化与转发该类消息
- **AND** MUST NOT 要求首批父视图必须完成完整 progress timeline 或增量状态语义

#### Scenario: observe prompt is rendered

- **WHEN** 系统向模型描述 `observe`
- **THEN** prompt MUST 明确它是同步非阻塞查询
- **AND** MUST 明确禁止在没有后续决策的情况下高频轮询

#### Scenario: close prompt is rendered

- **WHEN** 系统向模型描述 `close`
- **THEN** prompt MUST 明确它会级联关闭 child 子树
- **AND** MUST 明确它是"结束分支"的动作，而不是"探测状态"的动作

#### Scenario: idle completion does not trigger an extra confirmation loop

- **WHEN** child 已完成本轮工作并回到 `Idle`
- **THEN** guidance MUST 要求 child 通过 unified `send` 主动上报，或由 runtime fallback 接管
- **AND** MUST NOT 再追加一轮“是否完成、是否要发给 parent”的 LLM 追问
