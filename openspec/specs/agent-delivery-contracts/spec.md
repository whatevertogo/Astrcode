## Purpose

定义 child delivery 的稳定控制合同，包括父级交付、唤醒、关闭和观察的完整生命周期，以及父子协作交付的逐级冒泡语义。

## Requirements

### Requirement: Stable Agent Delivery And Control Contract

系统 SHALL 为 child delivery、observe、wake、close 暴露稳定控制合同，并在 child 终态出现时通过正式 delivery queue 与 parent wake 管线驱动父级后续执行。

#### Scenario: Deliver message to child agent

- **WHEN** 上层向某个 child agent 或 subrun 路由消息
- **THEN** 系统 SHALL 通过稳定控制接口完成投递
- **AND** 调用方 SHALL 不需要了解内部 mailbox 实现

#### Scenario: Wake suspended agent

- **WHEN** 上层请求唤醒可恢复的 agent 或 subrun
- **THEN** 系统 SHALL 通过稳定接口执行唤醒
- **AND** 失败 SHALL 明确暴露为领域错误

#### Scenario: Close agent subtree

- **WHEN** 上层请求关闭某个 agent 子树
- **THEN** 系统 SHALL 提供统一关闭合同
- **AND** 该合同 SHALL 明确返回关闭结果

#### Scenario: Observe agent execution

- **WHEN** 上层订阅某个 agent 或 subrun 的执行过程
- **THEN** 系统 SHALL 返回稳定观察流或稳定观察快照
- **AND** SHALL 不暴露内部事件总线协议

#### Scenario: Child completion wakes parent through delivery pipeline

- **WHEN** 子代理完成、失败或被关闭且需要向父级回流结果
- **THEN** 系统 SHALL 先持久化 delivery 所需信息并入队父级交付缓冲
- **AND** SHALL 尝试通过稳定 wake 接口启动父级后续执行

#### Scenario: Wake failure requeues delivery batch

- **WHEN** 父级 wake 提交失败或父级当前不可获取执行机会
- **THEN** 系统 SHALL 保留或重新排队对应 delivery batch
- **AND** MUST NOT 静默丢弃 child 终态回流

### Requirement: Parent delivery batch lifecycle

kernel 与 application SHALL 为 parent delivery batch 定义稳定生命周期，使 child 终态回流具备可重试与可观测行为。

#### Scenario: Delivery batch enters waking state

- **WHEN** 系统 checkout 一批父级交付用于 wake
- **THEN** 该批次进入"正在唤醒父级"的中间状态
- **AND** 在被 consume 或 requeue 前不得被重复消费

#### Scenario: Busy parent defers batch consumption

- **WHEN** 父级当前忙碌，无法立即开始 wake turn
- **THEN** 该批次保持或恢复为待重试状态
- **AND** MUST NOT 被提前 consume

#### Scenario: Successful wake consumes batch

- **WHEN** 父级 wake turn 成功接受并完成该批次
- **THEN** 系统从 parent delivery queue 中消费该批次

#### Scenario: Failed wake keeps batch retryable

- **WHEN** 父级 wake turn 提交失败或中途失败
- **THEN** 系统重新排队该批次
- **AND** SHALL 记录对应失败信号供观测使用

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
