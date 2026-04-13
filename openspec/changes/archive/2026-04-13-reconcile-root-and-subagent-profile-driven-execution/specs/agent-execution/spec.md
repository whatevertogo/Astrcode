## MODIFIED Requirements

### Requirement: 根代理执行

application App SHALL 提供 `execute_root_agent` 方法，将 API 请求转化为完整的 session turn。完整流程：参数解析 → profile 加载 → session 创建 → root agent 注册 → 异步执行。

#### Scenario: 执行指定 agent

- **WHEN** 调用 `execute_root_agent(agent_id, task, context, working_dir)`
- **THEN** 系统加载匹配的 agent profile，创建新 session，注册根 agent 到控制树，异步执行 turn，返回 `ExecutionAccepted`

#### Scenario: agent profile 不存在

- **WHEN** 指定的 agent_id 在 profile 注册表中不存在
- **THEN** 返回 `ApplicationError::NotFound` 错误

#### Scenario: agent 不支持根执行模式

- **WHEN** agent profile 的 mode 不允许根执行
- **THEN** 返回 `ApplicationError::InvalidArgument` 错误

### Requirement: 子代理执行

application App SHALL 提供子代理执行能力，支持 spawn/send/observe/close 四工具模型的完整执行路径，并在 spawn 时消费 working-dir 解析出的真实 profile。

#### Scenario: spawn 子代理

- **WHEN** 通过 `launch_subagent` 启动子代理
- **THEN** 系统先解析并校验子代理 profile，再创建子 session，注册子 agent 到控制树，异步执行子 turn，返回 `SubRunResult`

#### Scenario: 子代理完成返回结果

- **WHEN** 子代理 turn 执行完成
- **THEN** 系统将结果通过 `SubRunResult` 返回给调用方，并将结果推入父 agent 的 delivery 队列

#### Scenario: 无效 profile 不产生副作用

- **WHEN** 目标 subagent profile 不存在或不允许作为子代理执行
- **THEN** 返回业务错误
- **AND** MUST NOT 创建 child session 或注册子 agent

### Requirement: Agent Profile 加载与注册

application SHALL 支持按 working_dir 加载 scoped agent profile 注册表，并缓存结果，且 root/subagent 执行 SHALL 通过该能力消费正式 profile。

#### Scenario: 首次加载 profile

- **WHEN** 调用 `load_profiles_for_working_dir` 且缓存中无对应条目
- **THEN** 系统通过 adapter-agents 加载该目录的 agent profiles，缓存并返回

#### Scenario: 缓存命中

- **WHEN** 调用 `load_profiles_for_working_dir` 且缓存已有该目录的注册表
- **THEN** 直接返回缓存的注册表

#### Scenario: 执行链使用统一 profile 事实源

- **WHEN** root 或 subagent 执行需要确定目标 agent
- **THEN** 系统 SHALL 通过 scoped profile 注册表解析目标 profile
- **AND** MUST NOT 让 server 路由或 application 编排层各自维持不同的 profile 语义
