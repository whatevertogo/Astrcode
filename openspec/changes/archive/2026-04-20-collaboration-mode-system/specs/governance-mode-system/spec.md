## ADDED Requirements

### Requirement: governance mode catalog SHALL support builtin and plugin-defined modes through stable IDs

系统 SHALL 通过开放式 mode catalog 管理执行治理模式，而不是依赖封闭枚举。每个 mode MUST 由稳定 `mode id` 标识，并可由 builtin 或插件注册。

#### Scenario: builtin execute mode is available by default

- **WHEN** 系统创建一个新 session，且没有显式 mode 事件
- **THEN** 系统 SHALL 解析到 builtin `code` mode
- **AND** 该默认 mode 的治理行为 SHALL 与当前默认执行行为保持等价

#### Scenario: plugin-defined mode joins the same catalog

- **WHEN** 一个插件在 bootstrap 或 reload 成功注册自定义 mode
- **THEN** 该 mode SHALL 出现在统一的 mode catalog 中
- **AND** 系统 SHALL 继续用与 builtin mode 相同的解析与编译流程消费它

### Requirement: governance mode SHALL compile to a turn-scoped execution envelope

系统 SHALL 在 turn 边界把当前 mode 编译为 `ResolvedTurnEnvelope`。该 envelope MUST 至少包含当前 turn 的 capability surface、prompt declarations、execution limits、action policies 与 child policy。

#### Scenario: plan mode compiles a restricted capability surface

- **WHEN** 当前 session 的 mode 为一个只读规划型 mode
- **THEN** 系统 SHALL 为该 turn 编译出收缩后的 capability router
- **AND** 当前 turn 模型可见的工具集合 SHALL 与该 router 保持一致

#### Scenario: execute mode compiles the full default envelope

- **WHEN** 当前 session 的 mode 为 builtin `code`
- **THEN** 系统 SHALL 编译出与当前默认执行行为等价的 envelope
- **AND** SHALL NOT 因引入 mode 而额外改变 turn loop 语义

### Requirement: mode capability selection SHALL be resolved against the current capability semantic model

mode 的能力选择 MUST 建立在当前 `CapabilitySpec` / capability router 之上，而不是维护平行工具注册表。mode selector SHALL 至少支持基于名称、kind、side effect 或 tag 的投影。

#### Scenario: selector filters against current capability surface

- **WHEN** 某个 mode 使用 tag 或 side-effect selector 选择能力
- **THEN** 系统 SHALL 基于当前 capability surface 中的 `CapabilitySpec` 解析可见能力
- **AND** SHALL NOT 通过独立 mode 工具目录重建另一份真相

#### Scenario: plugin capability is governed by the same selectors

- **WHEN** 当前 capability surface 同时包含 builtin 与插件工具
- **THEN** mode selector SHALL 对它们一视同仁地解析
- **AND** SHALL NOT 因来源不同而走不同治理路径

### Requirement: session SHALL persist the current mode as an event-driven projection

系统 SHALL 通过 durable 事件记录 session 当前 mode 的变更，并在 `session-runtime` 内维护当前 mode 的投影缓存。

#### Scenario: new session replays without mode events

- **WHEN** 一个旧 session 的事件流中不存在 mode 变更事件
- **THEN** replay 结果 SHALL 回退到 builtin `execute` mode

#### Scenario: mode change survives replay

- **WHEN** session 已经追加过一次有效的 mode 变更事件
- **THEN** 会话重载或回放后 SHALL 恢复为该最新 mode
- **AND** 后续 turn SHALL 继续使用该 mode 的 envelope 编译结果

### Requirement: mode transition SHALL be validated through a unified governance entrypoint

所有 mode 切换请求 MUST 经过统一治理入口校验 target mode、transition policy 与 entry policy，然后再由 `session-runtime` 应用 durable 变更。

#### Scenario: invalid transition is rejected before runtime mutation

- **WHEN** 一个切换请求的目标 mode 不满足当前 mode 的 transition policy
- **THEN** 系统 SHALL 在追加任何 durable 事件前拒绝该请求

#### Scenario: valid transition applies on the next turn

- **WHEN** 当前 turn 中途发生一次合法 mode 切换
- **THEN** 当前 turn 的执行 envelope SHALL 保持不变
- **AND** 新 mode SHALL 从下一次 turn 开始生效

### Requirement: governance mode SHALL constrain orchestration inputs without replacing the runtime engine

governance mode 可以约束 prompt program、能力面、委派策略与行为入口，但 MUST NOT 直接替换 `run_turn`、tool cycle、streaming path 或 compaction 算法。

#### Scenario: plugin mode customizes prompt without replacing loop

- **WHEN** 一个插件 mode 定义了自定义 prompt program
- **THEN** 系统 SHALL 只把它编译为 envelope 中的 prompt declarations
- **AND** SHALL 继续使用同一套通用 turn loop 执行该 turn

#### Scenario: mode-specific loop implementations are forbidden

- **WHEN** 实现一个新的 governance mode
- **THEN** 系统 SHALL NOT 要求新增独立的 `run_<mode>_turn` 或等价 loop 实现

### Requirement: child sessions SHALL derive their initial governance mode from child policy

子 session 的初始治理模式 MUST 由父 turn 的 resolved child policy 推导，而不是简单继承父 session 的 mode 标签。

#### Scenario: parent plan mode launches an execute-capable child by policy

- **WHEN** 父 session 当前处于规划型 mode，且其 child policy 允许子分支使用执行型 mode
- **THEN** 新 child session SHALL 按 child policy 初始化为对应 child mode
- **AND** SHALL NOT 因父 mode 是 plan 而被强制继承为同名 mode

#### Scenario: child delegation is disabled by current mode

- **WHEN** 当前 mode 的 child policy 禁止创建新的 child 分支
- **THEN** 当前 turn SHALL 不向模型暴露新的 child delegation 行为入口
- **AND** SHALL 在 delegation surface 中反映该约束

### Requirement: mode catalog SHALL be assembled during bootstrap and updated on reload

builtin mode catalog MUST 在 server bootstrap 阶段通过 `GovernanceBuildInput` 装配，插件 mode 在 bootstrap 或 reload 时注册到同一 catalog。reload 时，mode catalog 的替换 SHALL 与能力面替换保持原子性。

#### Scenario: builtin modes are available after bootstrap

- **WHEN** server bootstrap 完成
- **THEN** `execute`、`plan`、`review` 等 builtin mode SHALL 已在 catalog 中注册
- **AND** 无需任何额外配置即可使用

#### Scenario: plugin mode joins catalog during bootstrap

- **WHEN** 一个插件在 bootstrap 握手阶段声明了自定义 mode
- **THEN** 该 mode SHALL 出现在统一 catalog 中
- **AND** 如果 mode spec 校验失败，SHALL 整批拒绝该插件的所有 mode

#### Scenario: reload atomically swaps mode catalog and capability surface

- **WHEN** runtime reload 触发
- **THEN** mode catalog 替换 SHALL 与 capability surface 替换在同一原子操作中完成
- **AND** reload 失败时 SHALL 继续使用旧的 mode catalog（与当前能力面回滚策略一致）

#### Scenario: running sessions are unaffected by catalog reload

- **WHEN** reload 发生时有 session 正在执行
- **THEN** 已在执行的 turn SHALL 使用 reload 前的 envelope
- **AND** 仅在下一 turn 开始时使用新的 catalog 编译 envelope

### Requirement: mode change SHALL be recorded as a durable event in session event log

mode 变更 MUST 通过 durable 事件记录到 session 事件流。`StorageEventPayload` SHALL 增加对应的变体（如 `ModeChanged`），确保 mode 变更可回放、可审计。

#### Scenario: mode change appends a ModeChanged event

- **WHEN** session 的 mode 成功切换
- **THEN** 系统 SHALL 追加一个 `ModeChanged { from: ModeId, to: ModeId }` 事件到 session event log
- **AND** 该事件 SHALL 包含足够信息用于回放和审计

#### Scenario: old session replay falls back to default mode

- **WHEN** 一个旧 session 的事件流中不包含 `ModeChanged` 事件
- **THEN** replay 结果 SHALL 回退到 builtin `execute` mode
- **AND** 行为 SHALL 与当前默认行为等价

#### Scenario: mode change survives replay

- **WHEN** session 事件流包含一个或多个 `ModeChanged` 事件
- **THEN** replay 后 SHALL 恢复为最新事件指定的 mode
- **AND** 后续 turn SHALL 使用该 mode 编译 envelope

### Requirement: session state SHALL maintain current mode as a projection from event log

`SessionState` SHALL 维护当前 mode 的投影缓存，该投影从事件流中 `ModeChanged` 事件增量计算得出。

#### Scenario: SessionState exposes current mode projection

- **WHEN** session state 需要知道当前 mode
- **THEN** 它 SHALL 从投影缓存中读取当前 `ModeId`
- **AND** 投影更新 SHALL 在 `translate_store_and_cache` 中通过事件驱动完成

#### Scenario: AgentState projector handles ModeChanged events

- **WHEN** `AgentStateProjector.apply()` 接收到 `ModeChanged` 事件
- **THEN** `AgentState` SHALL 更新其 mode 字段
- **AND** 后续 `project()` 调用 SHALL 反映最新 mode

### Requirement: collaboration audit facts SHALL include mode context

`AgentCollaborationFact` 记录的协作审计事件 MUST 包含当前 mode 上下文，使审计链路能追溯到 mode 治理决策。

#### Scenario: collaboration fact records active mode at action time

- **WHEN** 系统记录一个 `AgentCollaborationFact`（如 spawn 或 send）
- **THEN** 该事实 SHALL 包含当前 session 的 `mode_id`
- **AND** 审计查询 SHALL 能按 mode 过滤协作事实

#### Scenario: mode transition during turn does not affect audit context

- **WHEN** turn 执行中途发生 mode 变更（下一 turn 生效）
- **THEN** 当前 turn 内的协作事实 SHALL 使用 turn 开始时的 mode
- **AND** SHALL NOT 因 mode 变更导致同一 turn 内的审计上下文不一致

### Requirement: mode observability SHALL support monitoring and diagnostics

mode 系统的运行状态 SHALL 可观测，包括当前活跃 mode、mode 变更历史、以及 mode 编译的 envelope 摘要。

#### Scenario: observability snapshot includes current mode

- **WHEN** `ObservabilitySnapshotProvider` 采集快照
- **THEN** 快照 SHALL 包含 session 当前 mode ID
- **AND** SHALL 包含最近 mode 变更的时间戳

#### Scenario: envelope compilation diagnostics are available

- **WHEN** envelope 编译产生异常结果（如空能力面）
- **THEN** 系统 SHALL 记录诊断信息
- **AND** 该信息 SHALL 可通过 observability 接口查询
