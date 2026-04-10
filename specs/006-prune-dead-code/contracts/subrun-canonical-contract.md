# Contract: Subrun Canonical Contract

本合同定义清理后 subrun 领域的唯一正式表达。

## 1. Canonical Status

- canonical 状态模型：`AgentStatus`
- 允许值：
  - `Pending`
  - `Running`
  - `Completed`
  - `Cancelled`
  - `Failed`
  - `TokenExceeded`
- `TokenExceeded` 是正式终态，不得折叠成 `Completed`

## 2. Canonical Receipt

- canonical execution receipt：`ExecutionAccepted`
- 字段：
  - `session_id`
  - `turn_id`
  - `agent_id: Option<String>`
  - `branched_from_session_id: Option<String>`

规则：

- `core` 与 `runtime` 内部不能再并存第二套 receipt 类型
- server 如需对 route 做 DTO 投影，必须基于该 canonical receipt

## 3. Canonical Lineage Owner

- canonical lineage owner：`SubRunHandle`
- `parent_turn_id` 为必填
- `SubRunDescriptor` 删除
- descriptorless / downgrade 读取合同退出主线

## 4. Canonical Event Context Projection

- subrun 场景的 `AgentEventContext` 必须可从 `&SubRunHandle` 直接构造
- 保留 `sub_run()` 工厂方法，但它不再是 handle 场景的默认入口

## 5. Canonical Control Owner

- root prompt / root execute 仍由 `ExecutionOrchestrationBoundary` 负责
- `launch_subagent`、`get_subrun_handle`、close/cancel 等 live child control 由 `LiveSubRunControlBoundary` 负责

## 6. Child Reference Contract

- `ChildAgentRef` 保留 identity / lineage / status / canonical `open_session_id`
- `openable` 删除
- `ChildSessionNotification` 与 protocol DTO 外层不得再重复 `open_session_id`

## 7. Protocol Status Contract

- child/subrun 相关 protocol 状态必须使用独立 DTO 枚举
- DTO 枚举值域必须与 canonical `AgentStatus` 一一对应
- server / frontend 不得再依赖字符串匹配理解状态

## 8. Prompt Metrics Contract

- `PromptMetrics` 的共享字段必须提取为 `PromptMetricsPayload`
- storage event、agent event、protocol event 可以保留各自 envelope，但不得各自维护完整重复字段清单

## 9. Compaction Mapping Contract

- `Reactive` 只属于 runtime / hook 内部 compaction reason
- durable `CompactTrigger` 保持正式 trigger 集合
- internal reason -> durable trigger 只有一条集中映射定义
