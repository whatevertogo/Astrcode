## MODIFIED Requirements

### Requirement: `session-runtime` 是唯一会话真相面

`session-runtime` crate SHALL 统一负责会话生命周期与执行真相，包括：

- session 目录
- session actor / state
- session query / replay / history view
- turn loop
- interrupt / replay / branch
- observe / mailbox / routing
- durable event append
- session catalog 广播
- token budget 驱动的单 turn 自动续写
- turn 级 observability 汇总

#### Scenario: SessionRuntime 持有会话目录

- **WHEN** 检查 `SessionRuntime` 核心结构
- **THEN** 它持有 session registry（例如 `DashMap<SessionId, Arc<SessionActor>>`）

#### Scenario: SessionRuntime 提供会话查询与执行入口

- **WHEN** `server` 或 `application` 需要列 session、读取 history、replay 事件或提交 prompt
- **THEN** 统一通过 `SessionRuntime` 暴露的 query / mutation 入口完成
- **AND** 不再由 `application` 自己维护内存态 session history

#### Scenario: kernel 不再持有会话真相

- **WHEN** 检查 `kernel` crate
- **THEN** 不存在 session actor 目录或 session state 真相容器

### Requirement: 会话执行构造逻辑归 `session-runtime`

`build_agent_loop`、`LoopRuntimeDeps`、`AgentLoop`、`TurnRunner` SHALL 位于 `session-runtime/turn` 或 `session-runtime/factory`，不在 `kernel`。

#### Scenario: turn 由 session-runtime 完整驱动

- **WHEN** `application` 请求执行 turn
- **THEN** 通过 `SessionRuntime::run_turn(...)`（或等价入口）驱动完整执行

#### Scenario: turn 内预算与观测不泄漏到 application

- **WHEN** `application` 发起一次 prompt 提交
- **THEN** token budget、continue nudge、turn 级 observability 由 `session-runtime` 内部处理
- **AND** `application` 不拥有 turn 内循环策略

#### Scenario: kernel 不持有 turn 构造实现

- **WHEN** 检查 `kernel` 模块
- **THEN** 不存在 `build_agent_loop` 或 `LoopRuntimeDeps`
