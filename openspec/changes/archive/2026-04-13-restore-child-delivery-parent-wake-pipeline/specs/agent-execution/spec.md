## MODIFIED Requirements

### Requirement: 子代理执行

application App SHALL 提供子代理执行能力，支持 spawn/send/observe/close 四工具模型的完整执行路径，并在子代理终态时通过 parent delivery 管线把结果回流到父级。

#### Scenario: spawn 子代理

- **WHEN** 通过 `launch_subagent` 启动子代理
- **THEN** 系统创建子 session，注册子 agent 到控制树，异步执行子 turn，返回 `SubRunResult`

#### Scenario: 子代理完成返回结果

- **WHEN** 子代理 turn 执行完成
- **THEN** 系统将结果通过 `SubRunResult` 返回给调用方，并将结果推入父 agent 的 delivery 队列

#### Scenario: 子代理终态触发父级推进

- **WHEN** 子代理产生 Delivered、Failed 或 Closed 等终态通知
- **THEN** 系统 SHALL 通过 parent delivery queue 与 wake 调度推动父级后续执行

#### Scenario: wake 失败时保持可重试

- **WHEN** parent wake 失败
- **THEN** 系统 SHALL 将未消费的 delivery batch 保留为可重试状态
- **AND** MUST NOT 将该 batch 视为已成功消费
