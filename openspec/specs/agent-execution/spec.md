## Requirements

### Requirement: 根代理执行

application App SHALL 提供 `execute_root_agent` 方法，将 API 请求转化为完整的 session turn。完整流程：参数解析 → profile 加载 → session 创建 → root agent 注册 → 治理面组装 → resolved limits 持久化 → 异步执行。

#### Scenario: 执行指定 agent

- **WHEN** 调用 `execute_root_agent(kernel, session_runtime, profiles, governance, request, runtime_config)`
- **THEN** 系统校验参数（agent_id、working_dir、task 非空，control 无 manualCompact），加载匹配的 agent profile，校验 profile mode 为 Primary 或 All，创建新 session，注册根 agent 到控制树，组装治理面并持久化 resolved limits，合并 task + context，异步提交 prompt，返回 `ExecutionAccepted`（agent_id 设回请求值）

#### Scenario: agent profile 不存在

- **WHEN** 指定的 agent_id 在 profile 注册表中不存在
- **THEN** 返回 `ApplicationError::NotFound` 错误（含 profile id 和 working dir 信息）
- **AND** MUST NOT 创建 session 或注册 agent

#### Scenario: agent 不支持根执行模式

- **WHEN** agent profile 的 mode 不为 Primary 或 All（如 SubAgent only）
- **THEN** 返回 `ApplicationError::InvalidArgument` 错误（含 profile id 和 "root execution"）
- **AND** MUST NOT 创建 session 或注册 agent

#### Scenario: 执行控制被正式消费

- **WHEN** 调用方在执行请求中提供 `ExecutionControl`（含 maxSteps / tokenBudget）
- **THEN** 系统 SHALL 校验控制参数（validate），通过治理面解析为 resolved limits，持久化到 kernel 控制树

#### Scenario: context overrides 被拒绝

- **WHEN** 根执行请求携带非空的 `SubagentContextOverrides`
- **THEN** 返回 `ApplicationError::InvalidArgument("contextOverrides is not supported yet for root execution")`
- **AND** 默认空值（`SubagentContextOverrides::default()`）被静默接受

#### Scenario: 参数校验失败

- **WHEN** 请求的 agent_id、working_dir 或 task 为空/纯空白
- **THEN** 返回 `ApplicationError::InvalidArgument`，错误信息明确提及对应字段名（agentId / workingDir / task）
- **WHEN** control 中包含 manualCompact
- **THEN** 返回 `ApplicationError::InvalidArgument("manualCompact is not valid for root execution")`

---

### Requirement: 子代理执行

application App SHALL 提供子代理执行能力，支持 spawn/send/observe/close 四工具模型的完整执行路径，并在 spawn 时消费 working-dir 解析出的真实 profile。

#### Scenario: spawn 子代理

- **WHEN** 通过 `launch_subagent` 启动子代理
- **THEN** 系统校验参数（parent_session_id、parent_agent_id、working_dir、task 非空），校验 profile mode 为 SubAgent 或 All，组装子治理面（FreshChildGovernanceInput），构建 delegation metadata，创建独立 child session，在控制树中注册子 agent（`spawn_independent_child`），设置 lifecycle 为 Running，持久化 resolved limits 和 delegation metadata，合并 task + context，异步提交 prompt，记录 child_spawned 指标，返回 `ExecutionAccepted`（agent_id 设为 child handle）

#### Scenario: spawn 错误映射

- **WHEN** kernel 返回 `AgentControlError::MaxDepthExceeded`
- **THEN** 映射为 `ApplicationError::InvalidArgument`，提示复用已有 child（send/observe/close）
- **WHEN** kernel 返回 `AgentControlError::MaxConcurrentExceeded`
- **THEN** 映射为 `ApplicationError::Conflict`，提示等待或关闭已有 child
- **WHEN** kernel 返回 `AgentControlError::ParentAgentNotFound`
- **THEN** 映射为 `ApplicationError::NotFound`

#### Scenario: 无效 profile 不产生副作用

- **WHEN** 目标 subagent profile 不存在或不允许作为子代理执行（mode 不为 SubAgent 或 All）
- **THEN** 返回业务错误（NotFound 或 InvalidArgument）
- **AND** MUST NOT 创建 child session 或注册子 agent

#### Scenario: 子代理执行控制被正式消费

- **WHEN** 调用方在子代理执行请求中提供 capability_grant 或 parent_allowed_tools
- **THEN** 系统 SHALL 将 capability_grant 传入治理面组装，parent_allowed_tools 用于计算 effective allowed tools

---

### Requirement: Agent Profile 加载与注册

application SHALL 支持按 working_dir 加载 scoped agent profile 注册表，并缓存结果。

#### Scenario: 首次加载 profile

- **WHEN** 调用 `ProfileResolutionService::resolve(working_dir)` 且缓存中无对应条目
- **THEN** 系统通过 `ProfileProvider::load_for_working_dir` 加载该目录的 agent profiles，缓存（DashMap）并返回 `Arc<Vec<AgentProfile>>`

#### Scenario: 缓存命中

- **WHEN** 调用 `resolve(working_dir)` 且缓存已有该规范化路径的条目
- **THEN** 直接返回缓存的 `Arc<Vec<AgentProfile>>`，不再调用 provider

#### Scenario: 按 ID 查找 profile

- **WHEN** 调用 `find_profile(working_dir, profile_id)`
- **THEN** 系统先通过 `resolve` 获取 profile 列表，再按 id 匹配查找；未找到返回 `ApplicationError::NotFound`
- **AND** 即使缓存命中，profile 不存在仍返回 NotFound（缓存不替代业务校验）

#### Scenario: 全局 profile 加载

- **WHEN** 调用 `resolve_global()` 或 `find_global_profile(profile_id)`
- **THEN** 系统通过 `ProfileProvider::load_global` 加载全局 profiles，独立缓存（RwLock<Option<Arc<Vec<AgentProfile>>>>）

#### Scenario: 缓存失效

- **WHEN** 调用 `invalidate(working_dir)` → 该路径缓存失效
- **WHEN** 调用 `invalidate_global()` → 清除所有 scoped cache + 全局 cache
- **WHEN** 调用 `invalidate_all()` → 清除所有缓存
- **THEN** 后续请求重新从 provider 加载，已有快照不受影响（Arc 不可变语义）

---

### Requirement: Turn 租约互斥

application SHALL 确保同一 session 不会并发执行多个 turn。

#### Scenario: 正常获取租约

- **WHEN** session 当前无活跃 turn
- **THEN** `try_acquire_turn` 返回 `Acquired(turn_lease)`

#### Scenario: 租约冲突

- **WHEN** session 当前已有活跃 turn
- **THEN** 返回 `ApplicationError::Conflict` 错误

---

### Requirement: Agent 编排服务

application SHALL 提供 `AgentOrchestrationService`，作为 agent 子域的唯一服务入口，实现 `SubAgentExecutor` 和 `CollaborationExecutor` 两个 trait。

#### Scenario: spawn 通过 SubAgentExecutor.launch

- **WHEN** 调用 `launch(SpawnAgentParams, ToolContext)`
- **THEN** 系统确保 parent agent handle 存在（显式或隐式注册），构建 tool collaboration context，解析 subagent profile，执行 spawn budget 检查，调用 `launch_subagent`，启动 child turn terminal watcher，记录 collaboration fact（Spawn + Accepted），返回 `SubRunResult::Running`（含 handoff artifacts 和 Progress delivery）

#### Scenario: send/close/observe 通过 CollaborationExecutor

- **WHEN** 调用 `send(SendAgentParams, ToolContext)` → 路由到 `route_send`
- **WHEN** 调用 `close(CloseAgentParams, ToolContext)` → 路由到 `close_child`
- **WHEN** 调用 `observe(ObserveParams, ToolContext)` → 路由到 `observe_child`
- **THEN** 均通过 agent 子域专用 kernel/session 端口完成操作，记录 collaboration fact

#### Scenario: parent agent 自动注册

- **WHEN** ToolContext 中无显式 agent_id
- **THEN** 系统按 session 查找隐式 root handle（`root-agent:{session_id}`），未找到则注册隐式 root agent（profile_id = "default"）

---

### Requirement: 协作事实记录

application SHALL 在每次 spawn/send/observe/close/delivery 操作时记录结构化的 `AgentCollaborationFact`。

#### Scenario: fact 记录流程

- **WHEN** 执行协作操作
- **THEN** 系统构建 `CollaborationFactRecord`（含 action、outcome、session_id、turn_id、child handle、delivery_id、reason_code、latency_ms 等），转换为 `AgentCollaborationFact`，追加到 session 事件流，同时记录到 metrics

#### Scenario: 被拒绝的 spawn 也记录 fact

- **WHEN** spawn 因 profile 解析失败或 budget 耗尽被拒绝
- **THEN** 系统记录 Spawn + Rejected fact（含 reason_code 和 error summary），best-effort 不阻塞返回

---

### Requirement: 共享辅助函数

execution 模块 SHALL 提供以下共享辅助函数供 root 和 subagent 路径复用。

#### Scenario: merge_task_with_context

- **WHEN** context 非空非纯空白
- **THEN** 返回 `{context}\n\n{task}`
- **WHEN** context 为空或纯空白
- **THEN** 返回原始 task

#### Scenario: ensure_profile_mode

- **WHEN** profile mode 在允许列表中
- **THEN** 返回 Ok(())
- **WHEN** profile mode 不在允许列表中
- **THEN** 返回 `ApplicationError::InvalidArgument`，含 profile id 和执行类型名称

---

### Requirement: AgentOrchestrationError 错误类型

agent 子域 SHALL 使用 `AgentOrchestrationError` 枚举区分错误类别：InvalidInput、NotFound、Internal。

#### Scenario: 错误映射

- **WHEN** `AgentOrchestrationError` 需要映射为 `AstrError`
- **THEN** InvalidInput / NotFound → `AstrError::Validation`，Internal → `AstrError::Internal`
