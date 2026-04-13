## MODIFIED Requirements

### Requirement: `application` 提供子代理执行入口

`application` SHALL 提供正式的子代理执行入口，负责先解析并校验目标 profile，再创建子执行、协调全局控制并触发单 session turn，并接受显式执行控制参数作为可选输入。

#### Scenario: spawn 子代理

- **WHEN** 调用子代理执行入口并提供必要上下文
- **THEN** 系统先按 working-dir 解析目标 profile
- **AND** profile 校验通过后才创建子执行并返回可追踪结果

#### Scenario: 子代理完成后结果回流父级

- **WHEN** 子代理执行结束
- **THEN** 结果通过既有 delivery / control 机制回流父级
- **AND** 不在 `application` 内形成新的结果真相缓存

#### Scenario: 子代理执行控制被正式消费

- **WHEN** 调用方在子代理执行请求中提供 `maxSteps` 或 `tokenBudget`
- **THEN** 系统 SHALL 校验并消费这些控制参数
- **AND** SHALL 将其传递到正确的业务边界

#### Scenario: profile 非法时不创建 child session

- **WHEN** 目标 profile 不存在或 mode 不允许作为 subagent
- **THEN** 系统返回业务错误
- **AND** MUST NOT 创建 child session 或注册新的子 agent
