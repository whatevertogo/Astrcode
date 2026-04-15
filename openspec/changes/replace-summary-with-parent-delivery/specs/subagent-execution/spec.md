## MODIFIED Requirements

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
- **THEN** 系统 SHALL 将该消息进入 child mailbox 排队
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
