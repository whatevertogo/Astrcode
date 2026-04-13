## ADDED Requirements

### Requirement: Composer SHALL submit explicit execution controls

前端 composer MUST 通过稳定 API 合同提交执行控制，而不是把控制语义停留在本地 TODO、隐式文本约定或零散条件分支中。

#### Scenario: Composer submits execution options

- **WHEN** 用户指定 `tokenBudget`、`maxSteps` 或其他正式支持的执行控制
- **THEN** 前端 SHALL 通过显式 DTO 提交这些控制
- **AND** 服务端 SHALL 按业务边界校验并消费这些字段

#### Scenario: Unsupported control is rejected explicitly

- **WHEN** 用户提交当前不支持的执行控制
- **THEN** 系统 SHALL 返回明确业务错误
- **AND** MUST NOT 静默忽略该控制

### Requirement: Busy-session control requests SHALL have stable handling

会话运行中收到的控制请求 MUST 通过正式语义处理，而不是只在前端本地直接拒绝。

#### Scenario: Compact requested during active turn

- **WHEN** 当前 session 正在执行且用户请求手动 compact
- **THEN** 系统 SHALL 通过正式控制路径处理该请求
- **AND** 用户 SHALL 能获知该请求是被延迟执行还是被显式拒绝

#### Scenario: Control handling survives reconnect

- **WHEN** 控制请求已被系统接受且前端随后重连或切换视图
- **THEN** 控制请求状态 SHALL 仍以服务端事实为准
- **AND** MUST NOT 只依赖前端本地内存
