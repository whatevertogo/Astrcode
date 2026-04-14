## Purpose

统一定义子代理执行入口与关闭观察的稳定行为，让子代理生命周期始终通过统一应用控制链路。

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

### Requirement: 向子代理追加消息 MUST 走稳定业务入口

向既有 child 追加下一步任务 MUST 通过正式业务入口完成，并遵循 direct-child 所有权与 mailbox 排队语义。

#### Scenario: 向 idle child 发送下一步任务

- **WHEN** 直接父代理向处于 `Idle` 的 child 发送新消息
- **THEN** 系统 SHALL 复用该 child 的既有上下文
- **AND** SHALL 以正式业务入口启动或恢复后续执行

#### Scenario: 向 running child 发送消息

- **WHEN** 直接父代理向仍在运行中的 child 发送消息
- **THEN** 系统 SHALL 将该消息进入 child mailbox 排队
- **AND** MUST 明确区分"已入队但尚未处理"和"已被 child 消费"

#### Scenario: 向非 owned child 发送消息被拒绝

- **WHEN** 调用方尝试向非直接拥有的 child 发送消息
- **THEN** 系统 MUST 返回业务错误
- **AND** MUST NOT 伪造成功排队结果

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

`application` SHALL 使用统一的 child turn terminal finalizer 收口真正的 child work turn 的 terminal 结果，而不是按 spawn、resume 分散维护不同逻辑。

#### Scenario: spawn child turn reaches terminal

- **WHEN** child agent 的首轮 spawn turn 结束
- **THEN** 系统 MUST 通过统一 finalizer 投影 terminal outcome
- **AND** MUST 向直接父级写入 terminal delivery

#### Scenario: resumed child turn reaches terminal

- **WHEN** child agent 通过 idle-resume 继续执行并结束
- **THEN** 系统 MUST 通过同一 finalizer 投影 terminal outcome
- **AND** MUST 向直接父级写入 terminal delivery

### Requirement: wake turn MUST NOT auto-manufacture a new upward terminal delivery

parent-delivery wake turn 是消费 mailbox 的协调 turn，不属于新的 child work turn。

#### Scenario: wake turn reaches terminal

- **WHEN** child agent 因 parent-delivery wake 而开始新一轮 turn 并结束
- **THEN** 系统 MUST 只完成当前 mailbox batch 的 `acked / consume / requeue`
- **AND** MUST NOT 因为这个 wake turn 自动向更上一级写入新的 terminal delivery

### Requirement: terminal business failures MUST still be delivered upward

child turn 的业务终态若为 `Failed`、`Cancelled` 或 `TokenExceeded`，系统 SHALL 仍将其作为 terminal delivery 投递给直接父级。

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
