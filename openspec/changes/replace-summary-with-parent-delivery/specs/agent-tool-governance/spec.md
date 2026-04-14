## MODIFIED Requirements

### Requirement: four-tool collaboration MUST follow a single decision protocol

当系统同时暴露 `spawn / send / observe / close` 四个协作工具时，prompt guidance MUST 把它们表达为一套互斥的下一步决策协议，而不是四个彼此独立的说明书。若当前 child session 还暴露 `reply_to_parent`，系统 MUST 明确它是 child 向 direct parent 的正式回流动作，而不是把 `send` 扩展成双向消息。

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

#### Scenario: child-scoped upward reply guidance is rendered

- **WHEN** 当前 child session 可使用 `reply_to_parent`
- **THEN** guidance MUST 明确它用于向 direct parent 汇报 progress、completed、failed 或 close_request
- **AND** MUST 明确禁止把它用于跨树闲聊、重复粘贴全文结果或替代 `send`

### Requirement: collaboration tool prompts MUST stay action-oriented and low-noise

协作工具的 prompt metadata MUST 优先约束下一步动作，而不是重复解释底层 runtime 细节或鼓励无用思考。child upward reply contract 必须被明确说明，但不应把整套 delivery 实现细节塞进单个工具 description。

#### Scenario: send prompt is rendered

- **WHEN** 系统向模型描述 `send`
- **THEN** prompt MUST 强调"一次发送一条具体的下一步指令"
- **AND** MUST 明确禁止把 `send` 用作状态探测或模糊催促

#### Scenario: reply_to_parent prompt is rendered

- **WHEN** 系统向 child 描述 `reply_to_parent`
- **THEN** prompt MUST 明确要求在完成、失败或请求结束分支时通过该工具向 parent 发正式消息
- **AND** MUST 明确说明该消息会替代旧的 summary 驱动 handoff 语义

#### Scenario: progress does not expand first-wave UI scope

- **WHEN** 首批 `reply_to_parent` 合同已支持 `progress`
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
- **THEN** guidance MUST 要求 child 通过 `reply_to_parent` 主动上报，或由 runtime fallback 接管
- **AND** MUST NOT 再追加一轮“是否完成、是否要发给 parent”的 LLM 追问
