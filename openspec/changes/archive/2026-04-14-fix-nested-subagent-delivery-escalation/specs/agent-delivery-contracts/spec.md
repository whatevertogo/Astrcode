## MODIFIED Requirements

### Requirement: Deliver message to child agent

父子协作交付 SHALL 按直接父级逐级冒泡，不得把 child turn 的 terminal 收口绑定到整棵后代子树是否 settled。

#### Scenario: explicit child work turn can still report upward immediately

- **WHEN** `middle` 执行自己的一轮 child work turn
- **AND** 该 turn 在 `leaf` 等后代仍未 settled 时结束
- **THEN** 系统 MUST 仍允许 `middle` 立即向自己的直接父级汇报本轮 terminal 结果
- **AND** MUST NOT 等待整棵后代子树全部 settled

#### Scenario: middle spawns new child during wake

- **WHEN** `middle` 在处理 wake turn 时又产生新的 child work
- **THEN** 新 child 的完成 SHALL 通过后续独立 delivery/wake 周期继续回传
- **AND** 当前 wake turn MUST NOT 因为自身结束而自动向更上一级制造新的 terminal delivery
- **AND** 系统 MUST NOT 等待整棵后代子树全部 settled 才允许后续显式 child work turn 上报

#### Scenario: wake turn stays at the direct consumer boundary

- **WHEN** `leaf` 的 terminal delivery 唤醒 `middle`
- **AND** `middle` 完成这轮 wake turn
- **THEN** 系统 MUST 在 `middle` 侧完成当前 batch 的消费
- **AND** 新 child 的完成 SHALL 通过后续独立 delivery/wake 周期继续回传
- **AND** MUST NOT 自动继续为 `root` 生成一条新的 child terminal delivery

#### Scenario: route truth is explicit

- **WHEN** 系统向父侧 session 追加 child terminal notification
- **THEN** 路由落点 MUST 来自显式 parent routing context
- **AND** MUST NOT 从 `ChildAgentRef.session_id` 反推父侧落点
