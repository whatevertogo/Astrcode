## MODIFIED Requirements

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

## ADDED Requirements

### Requirement: 向子代理追加消息 MUST 走稳定业务入口

向既有 child 追加下一步任务 MUST 通过正式业务入口完成，并遵循 direct-child 所有权与 mailbox 排队语义。

#### Scenario: 向 idle child 发送下一步任务

- **WHEN** 直接父代理向处于 `Idle` 的 child 发送新消息
- **THEN** 系统 SHALL 复用该 child 的既有上下文
- **AND** SHALL 以正式业务入口启动或恢复后续执行

#### Scenario: 向 running child 发送消息

- **WHEN** 直接父代理向仍在运行中的 child 发送消息
- **THEN** 系统 SHALL 将该消息进入 child mailbox 排队
- **AND** MUST 明确区分“已入队但尚未处理”和“已被 child 消费”

#### Scenario: 向非 owned child 发送消息被拒绝

- **WHEN** 调用方尝试向非直接拥有的 child 发送消息
- **THEN** 系统 MUST 返回业务错误
- **AND** MUST NOT 伪造成功排队结果

### Requirement: observe 结果 MUST 提供决策友好的投影

`observe` 的返回结果 MUST 保留 lifecycle 与 turn outcome 等原始事实，同时提供非权威的决策投影，帮助上级代理判断应继续等待、发送下一步任务还是结束该分支。

#### Scenario: child is still running

- **WHEN** child 当前处于 `Running` 或存在未处理中的当前任务
- **THEN** `observe` 结果 MUST 明确表达“当前应继续等待”
- **AND** MUST NOT 把再次 `spawn` 或重复 `observe` 表达为默认推荐动作

#### Scenario: child is idle after completing work

- **WHEN** child 已完成最近一轮 turn 并回到 `Idle`
- **THEN** `observe` 结果 MUST 明确表达该 child 可被 `send` 复用或被 `close` 结束
- **AND** MUST 暴露最近 turn outcome、pending message 情况或等价事实，支持上级做出该选择
