# composer-execution-controls Specification

## Purpose
TBD - created by archiving change modernize-composer-execution-controls. Update Purpose after archive.
## Requirements
### Requirement: Composer SHALL submit explicit execution controls

所有交互式 compose surface，包括桌面端、浏览器端和消费 conversation surface 的正式终端客户端，MUST 通过稳定 API 合同提交执行控制，而不是把控制语义停留在本地 TODO、隐式文本约定、slash 命令分支或零散条件分支中。

#### Scenario: Composer submits execution options

- **WHEN** 用户在任一交互式 client surface 中指定 `tokenBudget`、`maxSteps` 或其他正式支持的执行控制
- **THEN** 客户端 SHALL 通过显式 DTO 提交这些控制
- **AND** 服务端 SHALL 按业务边界校验并消费这些字段

#### Scenario: Unsupported control is rejected explicitly

- **WHEN** 任一交互式 client surface 提交当前不支持的执行控制
- **THEN** 系统 SHALL 返回明确业务错误
- **AND** MUST NOT 静默忽略该控制

#### Scenario: Terminal slash command maps to explicit control contract

- **WHEN** 终端用户通过 `/compact` 或其他 execution command 触发控制请求
- **THEN** 终端前端 SHALL 把该请求映射到与图形前端一致的显式执行控制合同
- **AND** MUST NOT 通过本地文本替换、隐藏 prompt 注入或旁路接口伪造控制语义

### Requirement: Busy-session control requests SHALL have stable handling

会话运行中收到的控制请求 MUST 通过正式语义处理，而不是只在某一个 client 本地直接拒绝；其接受、延迟执行或拒绝状态都 MUST 以服务端事实为准，并能跨重连与 surface 切换继续观察到。

#### Scenario: Compact requested during active turn

- **WHEN** 当前 session 正在执行且任一 client 请求手动 compact
- **THEN** 系统 SHALL 通过正式控制路径处理该请求
- **AND** 用户 SHALL 能获知该请求是被延迟执行还是被显式拒绝

#### Scenario: Control state is server-observable across reconnect

- **WHEN** 控制请求已被系统接受且发起方随后重连、切换视图或改由另一种 interactive surface 恢复该会话
- **THEN** 控制请求状态 SHALL 仍以服务端事实为准
- **AND** MUST NOT 只依赖任一 client 本地内存
