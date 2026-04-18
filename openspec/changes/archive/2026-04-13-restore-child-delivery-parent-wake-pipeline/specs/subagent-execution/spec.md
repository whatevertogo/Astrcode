## MODIFIED Requirements

### Requirement: `application` 提供子代理执行入口

`application` SHALL 提供正式的子代理执行入口，负责创建子执行、协调全局控制并触发单 session turn，并接受显式执行控制参数作为可选输入。子代理终态 MUST 进入正式 delivery / wake 管线，而不是只停留在子会话本地结果。

#### Scenario: spawn 子代理

- **WHEN** 调用子代理执行入口并提供必要上下文
- **THEN** 系统创建子执行并返回可追踪结果

#### Scenario: 子代理完成后结果回流父级

- **WHEN** 子代理执行结束
- **THEN** 结果通过既有 delivery / control 机制回流父级
- **AND** 不在 `application` 内形成新的结果真相缓存

#### Scenario: 子代理执行控制被正式消费

- **WHEN** 调用方在子代理执行请求中提供 `maxSteps` 或 `tokenBudget`
- **THEN** 系统 SHALL 校验并消费这些控制参数
- **AND** SHALL 将其传递到正确的业务边界

#### Scenario: 父级繁忙时结果不会丢失

- **WHEN** 子代理执行结束但父级当前正在执行
- **THEN** 系统 SHALL 将结果保留在 parent delivery 缓冲中
- **AND** SHALL 在后续可执行时继续尝试 wake 父级
