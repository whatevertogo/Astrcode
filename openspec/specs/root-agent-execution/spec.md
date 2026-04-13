## Requirements

### Requirement: `application` 提供根代理执行入口

`application` SHALL 提供正式的根代理执行入口，将用户请求转化为一次完整的 session 执行。

#### Scenario: 执行根代理

- **WHEN** 调用根代理执行入口并提供 `agent_id`、`task`、`working_dir`
- **THEN** 系统创建或准备目标 session
- **AND** 返回可追踪的执行回执

#### Scenario: 非法输入在 application 被拒绝

- **WHEN** `agent_id`、`task` 或 `working_dir` 非法
- **THEN** `application` 直接返回业务错误
- **AND** 不把错误请求继续下推到 `session-runtime` 或 `kernel`

---

### Requirement: 根代理执行必须通过已解析 profile 驱动

根代理执行 SHALL 基于 working-dir 解析出的 agent profile 进行，而不是在执行过程中临时猜测 profile。

#### Scenario: profile 存在时执行

- **WHEN** 指定 agent 的 profile 可被解析
- **THEN** 系统基于该 profile 发起执行

#### Scenario: profile 不存在时失败

- **WHEN** 指定 agent 的 profile 不存在
- **THEN** 返回 `NotFound` 或等价业务错误
