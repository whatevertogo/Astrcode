## ADDED Requirements

### Requirement: Stable Agent Delivery And Control Contract

系统 SHALL 为 child delivery、observe、wake、close 暴露稳定控制合同。

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

