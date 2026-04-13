## Purpose

为根代理执行入口建立稳定、可追踪的入口语义，使上层调用方可以明确地控制一次完整会话执行的行为。

## MODIFIED Requirements

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
