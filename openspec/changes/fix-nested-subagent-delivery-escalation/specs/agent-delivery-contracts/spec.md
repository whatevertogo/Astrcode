## MODIFIED Requirements

### Requirement: Deliver message to child agent

父子协作交付 SHALL 按直接父级逐级冒泡，不得把 child turn 的 terminal 收口绑定到整棵后代子树是否 settled。

#### Scenario: leaf delivery wakes middle and middle replies to root

- **WHEN** `leaf` 的 terminal delivery 唤醒 `middle`
- **AND** `middle` 当前 turn 结束
- **THEN** 系统 MUST 先把 `middle` 当前这一轮的 terminal delivery 投递给 `root`
- **AND** MUST NOT 停在 `leaf -> middle`

#### Scenario: middle spawns new child during wake

- **WHEN** `middle` 在处理 wake turn 时又产生新的 child work
- **THEN** `middle` 当前这一轮结束后仍 MUST 立即向直接父级汇报本轮 terminal 结果
- **AND** 新 child 的完成 SHALL 通过后续独立 delivery/wake 周期继续回传
- **AND** 系统 MUST NOT 等待整棵后代子树全部 settled 才允许 `middle` 上报

#### Scenario: route truth is explicit

- **WHEN** 系统向父侧 session 追加 child terminal notification
- **THEN** 路由落点 MUST 来自显式 parent routing context
- **AND** MUST NOT 从 `ChildAgentRef.session_id` 反推父侧落点
