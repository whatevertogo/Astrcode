## Purpose

定义 agent-tool 协作效果的评估体系，包括原始协作事实的记录、策略上下文捕获、稳定效果评分卡和可消费的评估读模型。

## Core Types

### AgentCollaborationFact（core 层）

结构化协作事实，是 agent-tool 评估系统的原始事实层。

字段：
- `fact_id`: DeliveryId — 事实唯一标识（格式 `acf-{uuid}`）
- `action`: AgentCollaborationActionKind — 动作类型
- `outcome`: AgentCollaborationOutcomeKind — 结果类型
- `parent_session_id`: SessionId — 父 session
- `turn_id`: TurnId — 触发的 turn
- `parent_agent_id`: Option<AgentId> — 父 agent
- `child_identity`: Option<ChildExecutionIdentity> — 子 agent 身份
- `delivery_id`: Option<DeliveryId> — 关联的 delivery
- `reason_code`: Option<String> — 失败/拒绝原因码
- `summary`: Option<String> — 人类可读摘要
- `latency_ms`: Option<u64> — 延迟
- `source_tool_call_id`: Option<DeliveryId> — 来源工具调用 ID
- `mode_id`: Option<ModeId> — 当前 mode
- `governance_revision`: Option<String> — 治理策略版本
- `policy`: AgentCollaborationPolicyContext — 策略上下文

方法：
- `child_agent_id() -> Option<&AgentId>` — 从 child_identity 提取

### AgentCollaborationActionKind（core 层）

- `Spawn` — 启动子代理
- `Send` — 发送协作消息
- `Observe` — 观察子代理状态
- `Close` — 关闭子代理
- `Delivery` — 交付结果

### AgentCollaborationOutcomeKind（core 层）

- `Accepted` — 被接受
- `Reused` — 被复用
- `Queued` — 已排队
- `Rejected` — 被拒绝
- `Failed` — 失败
- `Delivered` — 已交付
- `Consumed` — 已消费
- `Replayed` — 已重放
- `Closed` — 已关闭

### AgentCollaborationPolicyContext（core 层）

记录协作动作发生时的策略上下文：
- `policy_revision`: String — 策略版本
- `max_subrun_depth`: usize — 最大子运行深度
- `max_spawn_per_turn`: usize — 每 turn 最大 spawn 数

### AgentCollaborationScorecardSnapshot（core 层）

Agent collaboration 评估读模型，全部由 raw collaboration facts 派生。

原始计数：
- `total_facts`: u64 — 总事实数
- `spawn_accepted`: u64 — 成功 spawn
- `spawn_rejected`: u64 — 被拒绝 spawn
- `send_reused`: u64 — send 复用（向 idle child 发送）
- `send_queued`: u64 — send 排队（向 running child 发送）
- `send_rejected`: u64 — 被拒绝 send
- `observe_calls`: u64 — observe 调用
- `observe_rejected`: u64 — 被拒绝 observe
- `observe_followed_by_action`: u64 — observe 后跟了实际行动
- `close_calls`: u64 — close 调用
- `close_rejected`: u64 — 被拒绝 close
- `delivery_delivered`: u64 — 已交付 delivery
- `delivery_consumed`: u64 — 已消费 delivery
- `delivery_replayed`: u64 — 已重放 delivery
- `orphan_child_count`: u64 — 孤儿子代理数

派生比率（基点，`Option<u64>`，无数据时为 None）：
- `child_reuse_ratio_bps` — child 复用率
- `observe_to_action_ratio_bps` — observe-to-action 比率
- `spawn_to_delivery_ratio_bps` — spawn-to-delivery 比率
- `orphan_child_ratio_bps` — 孤儿 child 比率
- `avg_delivery_latency_ms` — 平均 delivery 延迟
- `max_delivery_latency_ms` — 最大 delivery 延迟

### CollaborationFactRecord（application 层）

application 层构建事实记录的 builder 结构体（定义在 `crates/application/src/agent/context.rs`）：

字段：
- `action`, `outcome`, `session_id`, `turn_id`
- `parent_agent_id`: Option<String>
- `child`: Option<&SubRunHandle>
- `delivery_id`, `reason_code`, `summary`: Option<String>
- `latency_ms`: Option<u64>
- `source_tool_call_id`: Option<String>
- `policy`: Option<AgentCollaborationPolicyContext>
- `governance_revision`: Option<String>
- `mode_id`: Option<ModeId>

Builder 方法链：`.parent_agent_id()`, `.child()`, `.delivery_id()`, `.reason_code()`, `.summary()`, `.latency_ms()`, `.source_tool_call_id()`, `.policy()`, `.governance_revision()`, `.mode_id()`

## Requirements

### Requirement: agent collaboration facts MUST be recorded as structured server-side records

系统 MUST 为 agent-tool 的关键协作动作记录结构化原始事实，并保证这些事实来自 server-side 业务真相，而不是只存在于前端或临时内存状态。

记录流程（`record_collaboration_fact`）：
1. 从 `CollaborationFactRecord` 构建 `AgentCollaborationFact`
2. 通过 `session_runtime.append_agent_collaboration_fact` 追加到 session 事件流
3. 通过 `metrics.record_agent_collaboration_fact` 记录到指标系统

#### Scenario: collaboration actions occur

- **WHEN** 系统执行 `spawn`、`send`、`observe`、`close` 或 child delivery 相关流程
- **THEN** 系统 MUST 通过 `record_collaboration_fact` 记录对应的 `AgentCollaborationFact`
- **AND** 这些事实 MUST 至少包含 parent_session_id、turn_id、action、outcome、policy context

#### Scenario: collaboration action fails

- **WHEN** 协作动作因限制、所有权错误或执行失败而未成功完成
- **THEN** 系统 MUST 通过 `record_fact_best_effort` 记录失败事实（best-effort，不阻塞返回）
- **AND** MUST 保留 reason_code 和 summary 用于后续诊断

### Requirement: evaluation records MUST capture effective policy context

协作评估记录 MUST 同时包含生效中的策略上下文，以支持不同 prompt/runtime 策略之间的比较。

#### Scenario: collaboration fact is recorded

- **WHEN** 系统写入一条 agent collaboration fact
- **THEN** 记录 MUST 包含 `AgentCollaborationPolicyContext`（policy_revision、max_subrun_depth、max_spawn_per_turn）
- **AND** MUST 包含 governance_revision 用于跨版本比较

### Requirement: system MUST derive a stable effectiveness scorecard from raw facts

系统 MUST 基于原始协作事实生成稳定的诊断读模型 `AgentCollaborationScorecardSnapshot`，用于判断 agent-tool 是否创造了实际协作价值。

#### Scenario: scorecard is built

- **WHEN** 系统为某段运行窗口生成效果读模型
- **THEN** 读模型 MUST 能表达 spawn accepted/rejected、send reused/queued、observe calls、close calls、delivery delivered/consumed 等核心计数
- **AND** MUST 能表达 child_reuse_ratio、observe_to_action_ratio、spawn_to_delivery_ratio、orphan_child_ratio 等派生比率（基点）
- **AND** MUST 能表达 avg/max delivery latency
- **AND** MUST 明确区分"没有数据"（Option::None）与"结果为零"（Some(0)）

#### Scenario: raw facts are incomplete

- **WHEN** 某些协作事实来源尚未接线或不可用
- **THEN** 读模型的派生比率 MUST 为 None（显式反映缺口）
- **AND** MUST NOT 静默把缺失数据伪装成有效低值

### Requirement: evaluation read models MUST be consumable without replaying full transcripts

系统 MUST 提供稳定的评估读模型，避免开发者为了判断 agent-tool 效果而手工重扫整条 transcript 或原始事件流。

#### Scenario: developer reads collaboration effectiveness

- **WHEN** 开发者读取 `AgentCollaborationScorecardSnapshot`
- **THEN** 系统 MUST 返回稳定聚合后的纯数据结构
- **AND** DTO MUST NOT 承载新的业务逻辑

### Requirement: RuntimeMetricsRecorder provides narrow write interface

`RuntimeMetricsRecorder` trait SHALL 提供窄写入接口，业务层只通过它记录事实和指标，不反向依赖具体快照实现。

#### Scenario: record collaboration fact

- **WHEN** 调用 `record_agent_collaboration_fact(&AgentCollaborationFact)`
- **THEN** 该事实被纳入 scorecard 的聚合计算

#### Scenario: record child spawned

- **WHEN** 调用 `record_child_spawned()`
- **THEN** `execution_diagnostics.child_spawned` 递增

#### Scenario: record subrun execution

- **WHEN** 调用 `record_subrun_execution(duration_ms, outcome, step_count, estimated_tokens, storage_mode)`
- **THEN** `subrun_execution` 指标按 outcome 更新（total、completed、failed、cancelled、token_exceeded）
