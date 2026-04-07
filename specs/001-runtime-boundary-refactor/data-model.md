# Data Model: Runtime Boundary Refactor

## 1. `SubRunDescriptor`

表示一次子执行的 durable lineage 最小事实集。它只回答“这次子执行是谁、挂在哪个父 turn / parent agent 下、位于哪一层”，不承载运行态状态。

| Field | Type | Required on new writes | Notes |
|-------|------|------------------------|-------|
| `sub_run_id` | `String` | Yes | 稳定子执行域 ID；生命周期事件和 status 查询都以它为主键 |
| `parent_turn_id` | `String` | Yes | 触发这次子执行的父 turn |
| `parent_agent_id` | `Option<String>` | Yes for new subruns; `None` only for legacy reads | 用于表达父子 ownership，不再从 session mode 或 UI tree 反推 |
| `depth` | `u32` | Yes | 根 agent 的直接子执行为 `1`；孙级为 `2`，依此类推 |

**Validation**

- `depth >= 1`
- `sub_run_id` 在同一 session replay 范围内唯一
- 新写入事件中 `parent_turn_id` 不允许为空
- 新写入事件中 `parent_agent_id` 不允许因“共享 session”或“独立 session”差异而变化

## 2. `SubRunLifecycleRecord`

表示一次子执行生命周期事件的 durable 载荷。`SubRunStarted` 与 `SubRunFinished` 共用相同的 lineage + trigger 结构，终态事件再附带结果统计。

| Field | Type | Applies to | Notes |
|-------|------|------------|-------|
| `descriptor` | `Option<SubRunDescriptor>` | start + finish | 新写入必须存在；旧历史允许为空以支持反序列化 |
| `tool_call_id` | `Option<String>` | start + finish | 记录触发该子执行的工具调用；legacy 历史允许为空 |
| `agent_context` | `AgentEventContext` | start + finish | 保留 agent/profile/storage/child session 等通用上下文 |
| `resolved_overrides` | `ResolvedSubagentContextOverrides` | start | durable 记录最终继承结果 |
| `resolved_limits` | `ResolvedExecutionLimitsSnapshot` | start | durable 记录执行限制快照 |
| `result` | `SubRunResult` | finish | 终态结果 |
| `step_count` | `u32` | finish | 终态累计步数 |
| `estimated_tokens` | `u64` | finish | 终态累计 token 估计 |

**Lifecycle states**

1. `started`: 有 `descriptor`、`tool_call_id`、resolved snapshots
2. `running`: 只存在于 live overlay 或由“已 started 但未 finish”推导
3. `completed` / `aborted` / `failed` / `tokenExceeded`: 由 `SubRunFinished.result.status` 定义

## 3. `ExecutionLineageIndex`

表示从 durable lifecycle event 派生出来的 lineage read model。它不是新的事实源，只是 `SubRunDescriptor` 的内存索引。

| Field | Type | Notes |
|-------|------|-------|
| `by_sub_run_id` | `Map<String, LineageEntry>` | 子执行主索引 |
| `agent_to_sub_run` | `Map<String, String>` | 通过 child agent id 找 parent/child 关系 |
| `trigger_to_sub_run` | `Map<String, String>` | 用于 tool call 与 subrun 的一对一链路 |
| `legacy_gaps` | `Set<String>` | 标记缺少 `descriptor` 的 legacy subrun |

**Consumers**

- `runtime-execution` subrun status 查询
- `server` 的 `/history` / `/events` 过滤
- `frontend` 的 subrun thread/tree read model

## 4. `ExecutionResolutionContext`

表示一次执行应该看到哪份 agent profile 快照，以及这份快照由哪个工作上下文决定。

| Field | Type | Notes |
|-------|------|-------|
| `working_dir` | `PathBuf` | 根执行显式传入；session prompt 与 subrun 从上游继承 |
| `search_dirs` | `Vec<PathBuf>` | resolver 实际扫描的 builtin/user/project agent 目录 |
| `registry_revision` | `String` or monotonic token | 用于调试这次执行绑定的是哪份快照 |
| `watched_roots` | `Vec<AgentWatchPath>` | 用于热重载与 resolver 失效 |

**Validation**

- 根执行必须显式指定 `working_dir`
- 同一次 execution tree 内的 child execution 默认继承 parent resolution context
- resolver 不能回退到“进程 cwd”而不显式出现在 context 中

## 5. `ExecutionBoundary`

表示一个运行时职责区及其单一 owner。

| Field | Type | Notes |
|-------|------|-------|
| `name` | enum/string | `runtime-session` / `runtime-execution` / `runtime-agent-loop` / `runtime-agent-control` / `runtime façade` |
| `owner_responsibility` | `String` | 该边界拥有的唯一核心职责 |
| `depends_on` | `Vec<String>` | 允许的上游边界或 `core` trait |
| `public_surface` | `Vec<String>` | 对 server / tools / runtime 外部可见的入口 |
| `surfaces_to_delete` | `Vec<String>` | 迁移完成后必须删除的旧入口 |

## 6. `EventScopeDefinition`

表示子执行过滤时的作用域语义。

| Scope | Meaning | Legacy behavior |
|-------|---------|----------------|
| `self` | 仅返回 `sub_run_id == target` 的事件 | 始终可用 |
| `directChildren` | 仅返回父级为 target 的直接子执行事件 | 缺 lineage 时拒绝，不做猜测 |
| `subtree` | 返回 target 及其整棵后代树 | 缺 lineage 时拒绝，不做猜测 |

## 7. `MigrationStage`

表示一次可独立评审和合并的重构阶段。

| Field | Type | Notes |
|-------|------|-------|
| `stage_id` | `String` | 例如 `M1-protocol` |
| `goal` | `String` | 该阶段唯一目标 |
| `caller_inventory` | `Vec<Path>` | 直接受影响的调用方 |
| `prerequisites` | `Vec<String>` | 删除或切换前必须满足的条件 |
| `validation` | `Vec<String>` | 合并前必须跑通的命令与场景 |

## Relationships

- 一个 `SubRunLifecycleRecord` 必须指向一个 `SubRunDescriptor`；legacy 历史除外。
- 一个 `ExecutionLineageIndex` 由一组 `SubRunLifecycleRecord` 派生而来，不直接落盘。
- 一个 `ExecutionResolutionContext` 由一次根执行确定，并沿 execution tree 向下继承。
- 一个 `ExecutionBoundary` 在一个 `MigrationStage` 中只能由一个 owner 负责迁移。

