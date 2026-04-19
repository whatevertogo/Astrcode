## Purpose

定义 root agent 与 subrun 的稳定状态查询合同，确保运行中状态、终态结果与 launch-time capability snapshot 都能被一致查询与 durable 回放。

## Core Types

### SubRunHandle（core 层）

受控子会话的轻量运行句柄，是 subrun 运行时句柄与 lineage 核心事实的唯一 owner。

字段：
- `sub_run_id`: SubRunId — 稳定的子执行域 ID
- `agent_id`: AgentId — 运行时分配的 agent 实例 ID
- `session_id`: SessionId — 子会话写入所在的 session
- `child_session_id`: Option<SessionId> — 独立子会话的 child session id
- `depth`: usize — 在父子树中的深度
- `parent_turn_id`: TurnId — 触发该子会话的父 turn
- `parent_agent_id`: Option<AgentId> — 父 agent
- `parent_sub_run_id`: Option<SubRunId> — 父 sub-run
- `lineage_kind`: ChildSessionLineageKind — 谱系来源
- `agent_profile`: String — 绑定的 profile ID
- `storage_mode`: SubRunStorageMode — 存储模式
- `lifecycle`: AgentLifecycleStatus — 生命周期状态
- `last_turn_outcome`: Option<AgentTurnOutcome> — 最近一轮执行的结束原因
- `resolved_limits`: ResolvedExecutionLimitsSnapshot — 生效的 capability 限制快照
- `delegation`: Option<DelegationMetadata> — 责任分支与复用边界元数据

方法：
- `child_identity() -> ChildExecutionIdentity`
- `parent_ref() -> ParentExecutionRef`
- `child_ref_with_status(status) -> ChildAgentRef`
- `open_session_id() -> &SessionId`
- `child_session_id_value() -> Option<&SessionId>`

### SubRunStatus（core 层）

子执行对外可观察的正式状态。是 `SubRunResult` 的 canonical status projection。

变体：
- `Running` — 运行中
- `Completed` — 已完成
- `TokenExceeded` — Token 超限完成
- `Failed` — 失败
- `Cancelled` — 已取消

方法：
- `lifecycle() -> AgentLifecycleStatus` — Running → Running，其余 → Idle
- `last_turn_outcome() -> Option<AgentTurnOutcome>` — 各变体映射到对应 outcome
- `is_failed() -> bool`
- `label() -> &'static str` — 如 "running", "completed" 等

### SubRunResult（core 层，tag = "kind"）

子执行的业务结果枚举：
- `Running { handoff: SubRunHandoff }`
- `Completed { outcome: CompletedSubRunOutcome, handoff: SubRunHandoff }`
- `Failed { outcome: FailedSubRunOutcome, failure: SubRunFailure }`

方法：
- `status() -> SubRunStatus` — canonical status projection
- `handoff() -> Option<&SubRunHandoff>` — 从 Running/Completed 中提取
- `failure() -> Option<&SubRunFailure>` — 从 Failed 中提取

### SubRunStatusView（kernel 层）

子运行稳定状态快照（不暴露内部树结构），从 `SubRunHandle` 投影得到。

字段：
- `sub_run_id`: String
- `agent_id`: String
- `session_id`: String
- `child_session_id`: Option<String>
- `depth`: usize
- `parent_agent_id`: Option<String>
- `agent_profile`: String
- `lifecycle`: AgentLifecycleStatus
- `last_turn_outcome`: Option<AgentTurnOutcome>
- `resolved_limits`: ResolvedExecutionLimitsSnapshot
- `delegation`: Option<DelegationMetadata>

方法：
- `from_handle(handle: &SubRunHandle) -> Self` — 从 SubRunHandle 投影

### AgentLifecycleStatus（core 层）

代理运行时生命周期状态：
- `Pending` — 已注册但未开始
- `Running` — 正在执行
- `Idle` — 完成当前 turn，等待下一步指令
- `Cancelled` — 被取消

### ResolvedExecutionLimitsSnapshot（core 层）

解析后的执行限制快照：
- `allowed_tools`: Vec<String> — 允许的工具列表
- `max_steps`: Option<u32> — 最大步数

## Requirements

### Requirement: Stable Subrun Status Contract

系统 SHALL 为 root agent 和 subrun 暴露稳定、一致、可查询的状态合同。

#### Scenario: Query root agent status

- **WHEN** 上层请求查询某个 session 关联的根 agent 执行状态（`query_root_status(session_id)`）
- **THEN** 系统 SHALL 返回 `SubRunStatusView`（不暴露 agent_tree 内部节点结构）

#### Scenario: Query subrun status

- **WHEN** 上层请求查询某个 subrun 的当前状态（`query_subrun_status(agent_id)`）
- **THEN** 系统 SHALL 返回 `SubRunStatusView`，覆盖 Running/Idle lifecycle 和 Completed/Failed/Cancelled turn outcome

#### Scenario: Status survives internal refactor

- **WHEN** `kernel` 内部 agent tree 存储结构发生变化
- **THEN** 上层依赖的状态查询合同（`SubRunStatusView`）SHALL 保持稳定

#### Scenario: List all statuses

- **WHEN** 调用 `list_statuses()`
- **THEN** 返回所有注册 agent 的 `Vec<SubRunStatusView>`

### Requirement: subrun status SHALL expose launch-time resolved capability snapshots

系统 MUST 为 subrun 暴露 child 启动时已经求得的 resolved capability snapshot，避免调用方只能从 transcript 或最新配置反推 child capability。

#### Scenario: query running child status

- **WHEN** 上层查询一个运行中的 subrun 状态
- **THEN** 返回结果的 `resolved_limits` MUST 反映 child 的 launch-time capability surface（`allowed_tools` + `max_steps`）
- **AND** MUST NOT 反映当前全局 capability registry 的完整视图

#### Scenario: query completed child status

- **WHEN** 上层查询一个已经完成的 subrun 状态
- **THEN** 返回结果 MUST 仍然保留该 child 启动时的 `resolved_limits`
- **AND** 调用方 MUST 能据此解释该 child 为什么在运行期间能或不能调用某些工具

### Requirement: subrun lifecycle events SHALL persist launch-time capability projections

subrun 的生命周期 durable 事件 MUST 持久化 child 启动时的 capability projection，确保状态查询与回放不依赖临时内存重新计算。

#### Scenario: child launch is recorded

- **WHEN** 系统记录某个 child 的 launch 事件
- **THEN** `SubRunHandle.resolved_limits` MUST 来源于 child 启动前已经完成的 capability projection
- **AND** 通过 `set_resolved_limits` 持久化到 kernel 控制树

#### Scenario: status is rebuilt from durable history

- **WHEN** 系统基于 durable 事件重建 subrun 状态
- **THEN** 返回的 `resolved_limits` MUST 可从 kernel 控制树恢复（`get_handle` → `resolved_limits`）
- **AND** MUST NOT 依赖当前磁盘上的最新 profile 或最新 capability router 重新推断
