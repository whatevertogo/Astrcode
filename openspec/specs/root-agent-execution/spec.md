## Purpose

为根代理执行入口补齐稳定的控制语义边界，便于应用层与运行时在治理与故障处理层保持一致行为。
## Requirements
### Requirement: `application` 提供根代理执行入口

`application` SHALL 提供正式的根代理执行入口，将用户请求转化为一次完整的 session 执行，并接受显式执行控制参数（如 `maxSteps`、`tokenBudget`）作为可选输入。

#### Scenario: 执行根代理

- **WHEN** 调用根代理执行入口并提供 `agent_id`、`task`、`working_dir`
- **THEN** 系统创建或准备目标 session
- **AND** 返回可追踪的执行回执

#### Scenario: 非法输入在 application 被拒绝

- **WHEN** `agent_id`、`task`、`working_dir` 或显式执行控制参数非法
- **THEN** `application` 直接返回业务错误
- **AND** 不把错误请求继续下推到 `session-runtime` 或 `kernel`

#### Scenario: 显式执行控制参与根执行

- **WHEN** 调用方在根代理执行请求中提供 `maxSteps` 或 `tokenBudget`
- **THEN** 系统 SHALL 将这些控制作为正式输入向下传递
- **AND** SHALL NOT 仅停留在前端或协议 TODO 字段中

### Requirement: 根代理执行必须通过已解析 profile 驱动

根代理执行 SHALL 基于 working-dir 解析出的 agent profile 进行，而不是在执行过程中临时猜测 profile。

#### Scenario: profile 存在时执行

- **WHEN** 指定 agent 的 profile 可被解析
- **THEN** 系统基于该 profile 发起执行

#### Scenario: profile 不存在时失败

- **WHEN** 指定 agent 的 profile 不存在
- **THEN** 返回 `NotFound` 或等价业务错误

