## Purpose

定义 agent-tool 协作的 prompt 治理规则，确保四工具（spawn / send / observe / close）在 prompt 层面遵循统一的决策协议，降低无效协作与过度 fan-out。

## Requirements

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

### Requirement: spawn guidance MUST distinguish behavior template from task-scoped authorization

当 `spawn` 用于创建 child agent 时，协作 guidance MUST 明确 `profile` 用于选择行为模板，而 capability grant 用于限定本次任务授权。

#### Scenario: spawn prompt is rendered

- **WHEN** 系统向模型描述 `spawn`
- **THEN** prompt MUST 明确选 profile 是选行为模板
- **AND** MUST 明确 capability grant 才是本次任务能力范围的主要输入

#### Scenario: restricted child is launched

- **WHEN** child 以收缩后的 capability surface 启动
- **THEN** guidance MUST 明确 child 只会看到 launch-time resolved capability surface 允许的工具
- **AND** MUST NOT 把 profile 名称暗示成能力授权开关

### Requirement: spawn guidance SHALL keep reuse-first behavior under capability mismatch

引入 task-scoped capability grant 后，协作 prompt 仍 MUST 保持 reuse-first 协议，避免模型因为 capability 配置变细而默认回到过度 fan-out。

#### Scenario: existing idle child still matches the responsibility

- **WHEN** 当前已有一个 `Idle` child 拥有正确责任边界，且其生效工具集足以完成下一步工作
- **THEN** guidance MUST 继续优先推荐 `send`
- **AND** MUST NOT 因为 profile 或 grant 组合更多而默认再次 `spawn`

#### Scenario: existing child lacks required capability

- **WHEN** 当前 child 的生效工具集无法满足下一步工作
- **THEN** guidance MUST 允许通过新的 capability grant `spawn` 一个更合适的 child
- **AND** MUST 明确原因是 capability mismatch，而不是把 `Idle` 误表述成错误状态

### Requirement: shared collaboration protocol SHALL remain centralized while tool descriptions stay low-noise

四工具的 description MUST 优先表达单工具动作语义；共享的 child delegation 协议与 mode-level guidance MUST 保持在专门的共享 guidance surface 中，而不是在每个工具 description 中重复内联。

#### Scenario: shared collaboration guidance is rendered

- **WHEN** 当前 session 同时可使用 `spawn / send / observe / close`
- **THEN** 系统 MUST 提供统一的共享 collaboration guidance
- **AND** 该 guidance MUST 承担四工具的通用决策协议，而不是要求每个工具 description 各自重复解释

#### Scenario: spawn tool description is rendered

- **WHEN** 系统向模型描述 `spawn`
- **THEN** tool description MUST 保持动作导向与低噪音
- **AND** MUST NOT 在 description 内重复内联完整 delegation catalog 或 child execution contract

#### Scenario: send / observe / close descriptions are rendered

- **WHEN** 系统向模型描述 `send`、`observe` 或 `close`
- **THEN** 每个 description MUST 聚焦于该工具的一步动作与边界
- **AND** MUST NOT 重新解释整个 child delegation 心智模型

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

### Requirement: authoritative collaboration guidance SHALL be assembled outside adapter-owned static prompt code

协作 guidance 的 authoritative 来源 MUST 来自统一治理装配路径，而不是继续散落在 adapter 层的静态 builtin prompt 代码中。

#### Scenario: adapter renders but does not own collaboration truth

- **WHEN** 模型 prompt 中出现协作 guidance
- **THEN** `adapter-prompt` SHALL 只负责渲染该 guidance 对应的 `PromptDeclaration`
- **AND** SHALL NOT 继续把协作治理真相直接硬编码在 contributor 内作为唯一事实源

#### Scenario: multiple entrypoints receive consistent collaboration guidance

- **WHEN** root execution、普通 session submit 与 child execution 都需要协作 guidance
- **THEN** 它们 SHALL 从同一治理装配路径获得一致的协作声明
- **AND** SHALL NOT 因入口不同而依赖不同的硬编码文本来源

### Requirement: spawn guidance SHALL distinguish fresh, resumed, and restricted delegation modes

协作 guidance MUST 正式区分 fresh child、resumed child 与 restricted child 三种 delegation mode，并为每种 mode 提供不同的 briefing 规则。

#### Scenario: fresh child is planned

- **WHEN** 模型准备创建一个 fresh child 来承担新的责任边界
- **THEN** guidance MUST 明确要求提供完整任务背景、边界与交付物
- **AND** MUST NOT 将简短催促语句表达为充分的 fresh child briefing

#### Scenario: resumed child is planned

- **WHEN** 模型准备复用一个已有 responsibility continuity 的 child
- **THEN** guidance MUST 明确要求只发送下一条具体指令或澄清
- **AND** MUST NOT 把 resumed child 当成 fresh child 重新完整布置任务

#### Scenario: restricted child is planned

- **WHEN** 模型准备启动一个 capability 收缩后的 child
- **THEN** guidance MUST 明确要求任务分配服从该 child 的 capability surface
- **AND** MUST 明确建议在 capability mismatch 时换用更合适的 child 或当前 agent 自行完成工作
