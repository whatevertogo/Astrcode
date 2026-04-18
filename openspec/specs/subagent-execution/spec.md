## Purpose

统一定义子代理执行、追加消息、观察、关闭与终态收口的稳定业务合同，确保子代理生命周期、能力收缩与上行结果投递始终通过同一条应用控制链路完成。

## Requirements

### Requirement: `application` 提供子代理执行入口

`application` SHALL 提供正式的子代理执行入口，负责先解析并校验目标 profile，再创建子执行、协调全局控制并触发单 session turn，并接受显式执行控制参数作为可选输入。

#### Scenario: spawn 子代理

- **WHEN** 调用子代理执行入口并提供必要上下文
- **THEN** 系统先按 working-dir 解析目标 profile
- **AND** profile 校验通过后才创建子执行并返回可追踪结果

#### Scenario: 子代理完成后结果回流父级

- **WHEN** 子代理执行结束
- **THEN** 结果通过既有 delivery / control 机制回流父级
- **AND** 不在 `application` 内形成新的结果真相缓存

#### Scenario: 子代理执行控制被正式消费

- **WHEN** 调用方在子代理执行请求中提供 `maxSteps` 或 `tokenBudget`
- **THEN** 系统 SHALL 校验并消费这些控制参数
- **AND** SHALL 将其传递到正确的业务边界

#### Scenario: profile 非法时不创建 child session

- **WHEN** 目标 profile 不存在或 mode 不允许作为 subagent
- **THEN** 系统返回业务错误
- **AND** MUST NOT 创建 child session 或注册新的子 agent

### Requirement: 子代理关闭与观察走稳定业务入口

子代理的关闭与观察 SHALL 通过稳定业务入口访问，并且只允许直接父代理对其拥有的 child 执行这些操作。观察结果 MUST 同时暴露原始状态真相和足以支持下一步决策的稳定输入。

#### Scenario: 关闭直接子代理

- **WHEN** 直接父代理调用关闭入口
- **THEN** 业务入口协调 control/session 两侧完成 subtree 级联关闭
- **AND** 返回结果 SHALL 明确说明被关闭的 root agent 与级联影响范围

#### Scenario: 查询直接子代理状态

- **WHEN** 直接父代理调用观察入口
- **THEN** 返回与当前 control 真相一致的状态快照
- **AND** 返回结果 SHALL 包含足以支持下一步决策的稳定字段，而不要求调用方重扫整段 transcript

#### Scenario: 跨树或非直接父关系调用被拒绝

- **WHEN** 调用方尝试观察或关闭不属于自己的 child
- **THEN** 系统 MUST 返回业务错误
- **AND** MUST NOT 暴露目标 child 的内部状态

### Requirement: 向子代理或父代理发送协作消息 MUST 走统一 `send` 业务入口

统一 `send` SHALL 作为父子协作的唯一消息入口，但上下行语义必须按这次消息的方向和参数分支严格区分，不得退化成自由双向聊天。非 root 的中间层 agent 既可能向上回 parent，也可能向下发 child。

#### Scenario: parent sends next instruction to idle child

- **WHEN** 直接父代理向处于 `Idle` 的 child 调用 `send`
- **AND** 输入匹配 `agentId + message + context` 分支
- **THEN** 系统 SHALL 复用该 child 的既有上下文
- **AND** SHALL 以正式业务入口启动或恢复后续执行

#### Scenario: parent sends message to running child

- **WHEN** 直接父代理向仍在运行中的 child 调用 `send`
- **AND** 输入匹配 `agentId + message + context` 分支
- **THEN** 系统 SHALL 将该消息进入 child input queue 排队
- **AND** MUST 明确区分"已入队但尚未处理"和"已被 child 消费"

#### Scenario: child sends typed delivery to direct parent

- **WHEN** child 在自己的执行上下文中调用 `send`
- **AND** 输入匹配 typed upward delivery payload 分支
- **THEN** 系统 MUST 将其解释为 `child -> direct parent` 的正式消息
- **AND** MUST NOT 要求 child 再调用其它独立工具

#### Scenario: middle agent can send both upward and downward in one turn

- **WHEN** 一个中间层 agent 在同一轮里既需要把结果回给 direct parent，又需要继续给 direct child 派发任务
- **THEN** 系统 MUST 允许它在同一个 `send` 工具下分别走 upward 和 downward 两个参数分支
- **AND** MUST NOT 因为该 agent 同时是 child 和 parent 就把其中一个方向判为非法

#### Scenario: invalid ownership or cross-tree send is rejected

- **WHEN** 调用方尝试向非直接拥有的 child 发送消息，或 child 尝试向非 direct parent 上行
- **THEN** 系统 MUST 返回业务错误
- **AND** MUST NOT 伪造成功排队结果

#### Scenario: root cannot masquerade as child upstream sender

- **WHEN** root 或非 child 上下文尝试用 upward payload 调用 `send`
- **THEN** 系统 MUST 在 `application` 层前置拒绝
- **AND** MUST 打结构化 log 与 collaboration fact

### Requirement: observe 结果 MUST 提供决策友好的投影

`observe` 的返回结果 MUST 保留 lifecycle 与 turn outcome 等原始事实，同时提供非权威的决策投影，帮助上级代理判断应继续等待、发送下一步任务还是结束该分支。

#### Scenario: child is still running

- **WHEN** child 当前处于 `Running` 或存在未处理中的当前任务
- **THEN** `observe` 结果 MUST 明确表达"当前应继续等待"
- **AND** MUST NOT 把再次 `spawn` 或重复 `observe` 表达为默认推荐动作

#### Scenario: child is idle after completing work

- **WHEN** child 已完成最近一轮 turn 并回到 `Idle`
- **THEN** `observe` 结果 MUST 明确表达该 child 可被 `send` 复用或被 `close` 结束
- **AND** MUST 暴露最近 turn outcome、pending message 情况或等价事实，支持上级做出该选择

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

### Requirement: child turn terminal result MUST use a unified finalizer

`application` SHALL 使用统一的 child turn terminal finalizer 收口真正的 child work turn 的 terminal 结果，而不是按 spawn、resume 分散维护不同逻辑。显式 upward send 是主路径；未显式上报时，finalizer MUST 生成 deterministic fallback delivery。

#### Scenario: spawn child turn reaches terminal

- **WHEN** child agent 的首轮 spawn turn 结束
- **THEN** 系统 MUST 通过统一 finalizer 投影 terminal outcome
- **AND** MUST 向直接父级写入 terminal delivery

#### Scenario: resumed child turn reaches terminal

- **WHEN** child agent 通过 idle-resume 继续执行并结束
- **THEN** 系统 MUST 通过同一 finalizer 投影 terminal outcome
- **AND** MUST 向直接父级写入 terminal delivery

#### Scenario: explicit terminal send prevents duplicate synthesis

- **WHEN** child 在当前 child work turn 内已经通过 unified `send` 显式发送 `completed`、`failed` 或 `close_request`
- **THEN** finalizer MUST 复用该 turn 已存在的 terminal upward delivery 事实
- **AND** MUST NOT 再根据 `summary`、`final_reply_excerpt` 或其它二次摘要字段合成重复结果

#### Scenario: idle or terminal child without explicit reply gets fallback delivery

- **WHEN** child work turn 进入 terminal 或回到 `Idle`
- **AND** 当前 turn 尚未显式向 direct parent 回报 terminal 结果
- **THEN** finalizer MUST 根据最终 assistant output 或失败事实自动生成一条 deterministic fallback delivery
- **AND** MUST NOT 触发额外一轮“是否完成”的 LLM 追问

### Requirement: wake turn MUST NOT auto-manufacture a new upward terminal delivery

parent-delivery wake turn SHALL 被视为消费 input queue 的协调 turn，而不是新的 child work turn。

#### Scenario: wake turn reaches terminal

- **WHEN** child agent 因 parent-delivery wake 而开始新一轮 turn 并结束
- **THEN** 系统 MUST 只完成当前 input queue batch 的 `acked / consume / requeue`
- **AND** MUST NOT 因为这个 wake turn 自动向更上一级写入新的 terminal delivery

### Requirement: terminal business failures MUST still be delivered upward

child turn 的业务终态若为 `Failed`、`Cancelled` 或 `TokenExceeded`，系统 SHALL 仍将其作为 terminal delivery 投递给直接父级。该投递 MUST 使用 typed upward delivery，而不是 `summary` / `final_reply_excerpt` 对。

#### Scenario: child turn fails

- **WHEN** child turn 进入 `Failed`
- **THEN** 系统 MUST 生成失败态 terminal delivery
- **AND** 直接父级 MUST 能观察到该失败投影

#### Scenario: child turn is cancelled

- **WHEN** child turn 进入 `Cancelled`
- **THEN** 系统 MUST 生成关闭态 terminal delivery
- **AND** 直接父级 MUST 能观察到该关闭投影

### Requirement: finalizer failures MUST NOT fake successful consumption

如果统一 finalizer 自身失败，系统 SHALL 保持当前交付批次可重试，不得制造"上级已经成功收到结果"的假象。

#### Scenario: finalizer append fails

- **WHEN** finalizer 在追加 durable notification 之前或期间失败
- **THEN** 系统 MUST NOT 标记对应批次为已成功消费
- **AND** 上级 MUST 保留后续重试机会

### Requirement: subagent launch SHALL accept task-scoped capability grants

`application` 在 `spawn` child 时 MUST 支持任务级 capability grant，用于描述该 child 在本次任务里申请的最小能力范围，而不是要求调用方通过 `AgentProfile` 预组合出所有权限变体。

#### Scenario: spawn child with explicit capability grant

- **WHEN** 调用方在 `spawn` 时显式提供 capability grant
- **THEN** 系统 MUST 将该 grant 作为 child launch 的输入之一
- **AND** child 的运行时能力面 MUST 受该 grant 约束

#### Scenario: spawn child without capability grant

- **WHEN** 调用方未提供 capability grant
- **THEN** 系统 MUST 回退到父级可继承 capability surface 与 runtime availability 的交集
- **AND** MUST NOT 读取 profile 中的工具清单作为 child 权限真相

### Requirement: child resolved capability surface SHALL be derived from runtime capability truth

child 的 resolved capability surface MUST 由当前 capability router、父级可继承上界、任务级 grant 与 runtime availability 求交得到，而不是由 `AgentProfile` 直接决定。

#### Scenario: grant requests a subset of parent capabilities

- **WHEN** grant 申请的工具集合是父级当前可继承工具面的真子集
- **THEN** child 的 resolved capability surface MUST 收缩到该子集
- **AND** child MUST NOT 获得 grant 之外的额外工具

#### Scenario: grant requests unavailable or non-inheritable capability

- **WHEN** grant 申请了当前 runtime 不可用或父级不可继承的能力
- **THEN** 系统 MUST 在 child launch 时给出明确结果
- **AND** MUST NOT 悄悄让 child 获得超出上界的能力

#### Scenario: profile remains a behavior template

- **WHEN** child launch 解析目标 `AgentProfile`
- **THEN** profile SHALL 继续决定 child 的行为模板、模型偏好或默认 prompt 风格
- **AND** SHALL NOT 单独作为 child 工具授权的最终真相

### Requirement: child prompt assembly and tool execution SHALL share one filtered capability view

子代理的 prompt 组装与 tool execution MUST 基于同一份 filtered capability router / capability view，而不是分别读取不同来源的工具列表。

#### Scenario: prompt hides filtered-out tools

- **WHEN** child 的 resolved capability surface 过滤掉某些工具
- **THEN** child prompt 中的 capability 列表与 tool guidance MUST 不再暴露这些工具
- **AND** 这些工具的说明 MUST 随之从 child 的系统提示词可见面移除

#### Scenario: runtime rejects filtered-out tools

- **WHEN** child 后续规划到一个不在 resolved capability surface 内的工具
- **THEN** runtime MUST 将其视为不可用工具
- **AND** MUST NOT 因为全局 capability registry 中存在该工具就允许调用成功

#### Scenario: prompt and runtime remain aligned

- **WHEN** child 使用解析后的 capability surface 启动
- **THEN** prompt 中可见工具集合与 runtime 可执行工具集合 MUST 保持一致
- **AND** 系统 MUST NOT 出现“prompt 可见但 runtime 不可调”或“prompt 不可见但 runtime 可调”的漂移

### Requirement: child execution surfaces SHALL preserve responsibility continuity across launch and reuse

子代理执行链路 MUST 明确保留 child responsibility continuity，帮助上级代理区分“同一责任分支的继续推进”和“创建新的责任分支”。child 如果要向 parent 回流结果，也 MUST 维持这条 responsibility continuity，而不是通过 summary 投影重新构造另一套语义。

#### Scenario: fresh child starts a new responsibility branch

- **WHEN** 父级启动一个 fresh child
- **THEN** 系统 MUST 将其视为新的责任分支
- **AND** 后续观察与结果投影 MUST 能区分该分支与其他 child 的责任边界

#### Scenario: resumed child continues the same responsibility branch

- **WHEN** 父级向一个已有 child 发送后续具体指令
- **THEN** 系统 MUST 将其视为同一责任分支上的继续推进
- **AND** MUST NOT 在语义上把这次继续推进伪装成一个新的 fresh delegation

#### Scenario: child sends upward on the same responsibility branch

- **WHEN** child 通过 unified `send` 向 direct parent 汇报结果
- **THEN** 这条 upward delivery MUST 继续归属于同一责任分支
- **AND** MUST NOT 被 server 或前端重新包装成独立于该分支的 summary artifact

### Requirement: collaboration result projections SHALL expose reuse, close, and respawn decision hints

child 的 observe / terminal result projection MUST 在保留原始事实的同时，补充足以支持 reuse、close 或 respawn 决策的 advisory hints。

#### Scenario: idle child remains reusable for next step

- **WHEN** child 处于 `Idle`，且其当前 responsibility continuity 与 capability surface 仍足以完成预期下一步工作
- **THEN** 结果投影 MUST 明确表达该 child 可以继续通过 `send` 复用
- **AND** MUST 提供简短 reuse hint 或等价的推荐理由

#### Scenario: idle child no longer fits the work

- **WHEN** child 虽然处于 `Idle`，但其 responsibility 边界或 capability surface 已不适合下一步工作
- **THEN** 结果投影 MUST 明确表达存在 responsibility mismatch 或 capability mismatch
- **AND** MUST 允许上级代理据此决定关闭该分支或创建更合适的新 child

#### Scenario: restricted child completes work

- **WHEN** 一个 restricted child 完成最近一轮工作并回到 `Idle`
- **THEN** 结果投影 MUST 仍然保留该 child 的 capability-aware reuse 语义
- **AND** MUST NOT 把“已完成最近一轮工作”错误地表达为“后续任何工作都适合继续复用”
