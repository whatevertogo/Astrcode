# Data Model: 子智能体会话与缓存边界优化

本 feature 同时涉及 durable 实体、运行时桥接结构和投影视图。下面按三层拆开定义，避免把不同生命周期的数据混成一个“万能状态对象”。

## 1. `ChildSessionNode`（durable）

表示一个稳定的父子会话归属节点。它回答“这个 child session 属于谁、从哪里创建、当前是否还能被 resume”，但不直接承载 transcript 内容。

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `child_session_id` | `String` | Yes | 子会话稳定身份；新写入必须唯一 |
| `parent_session_id` | `String` | Yes | 父会话身份 |
| `parent_agent_id` | `Option<String>` | Yes | 顶层父 agent 可为空；普通子智能体必须存在 |
| `child_agent_id` | `String` | Yes | 运行时 targeting 使用的稳定 agent 身份 |
| `created_by_execution_id` | `String` | Yes | 首次创建该 child session 的执行实例 |
| `latest_execution_id` | `String` | Yes | 当前或最近一次执行实例 |
| `lineage_kind` | `spawn \| resume` | Yes | 本 feature 仅收紧 spawn / resume 语义 |
| `status` | `pending \| running \| completed \| failed \| cancelled \| terminated` | Yes | durable 生命周期状态 |

**Validation**

- 所有新写入的 `ChildSessionNode` 都必须使用独立 `child_session_id`，不得复用父会话 id。
- 任一 `ChildSessionNode` 都只代表独立子会话 durable 真相，不承载共享写入 fallback 语义。
- `latest_execution_id` 必须始终指向同一 `child_session_id` 下的最新执行实例。

## 2. `ChildExecutionInstance`（durable）

表示同一 child session 的某一次具体执行。spawn 和 resume 都会产生新的执行实例，但不会改变 `child_session_id`。

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `execution_id` | `String` | Yes | 本次执行唯一标识 |
| `child_session_id` | `String` | Yes | 归属子会话 |
| `trigger_kind` | `spawn \| resume` | Yes | 本次执行由首次创建还是恢复触发 |
| `status` | `pending \| running \| completed \| failed \| cancelled \| terminated` | Yes | 执行终态 |
| `resume_from_execution_id` | `Option<String>` | No | 若为 resume，记录来源执行 |
| `started_at` | `DateTime<Utc>` | Yes | 开始时间 |
| `ended_at` | `Option<DateTime<Utc>>` | No | 终止时间 |

**Validation**

- `trigger_kind=resume` 时必须沿用原 `child_session_id`，且 `execution_id` 必须是新的。
- 任一 `ChildExecutionInstance` 只能属于一个 `ChildSessionNode`。
- 同一 `child_session_id` 的 `status=running` 执行实例最多一个。

## 3. `ParentChildBoundaryFact`（durable）

表示父历史中对子智能体保留的边界事实，也是 `/history` 与 `/events` 投影的 durable 来源。

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `fact_id` | `String` | Yes | 边界事实唯一标识 |
| `parent_session_id` | `String` | Yes | 落入的父会话 |
| `child_session_id` | `String` | Yes | 关联的子会话 |
| `execution_id` | `String` | Yes | 对应执行实例 |
| `kind` | `started \| resumed \| delivered \| completed \| failed \| cancelled \| lineage_mismatch` | Yes | 事实类型 |
| `summary` | `String` | Yes | 父视图可读摘要 |
| `open_session_id` | `String` | Yes | 供前端/服务端打开子会话 |
| `diagnostic_code` | `Option<String>` | No | 失败或不一致时的稳定诊断码 |
| `created_at` | `DateTime<Utc>` | Yes | 生成时间 |

**Validation**

- `ParentChildBoundaryFact` 不得包含子会话完整 transcript、工具原始输出或 `ReactivationPrompt` 文本。
- `kind=lineage_mismatch` 时 `diagnostic_code` 必填。
- 相同交付不得生成重复的可消费边界事实。

## 4. `ResumeVisibleState`（重建结果，不要求单独 durable 落盘）

表示 resume 前需要从 child session durable 历史中重建出来的“下一轮执行可见状态”。它是恢复语义的目标，不要求必须固化为某个统一结构体。

| Component | Required | Notes |
|-----------|----------|-------|
| `message_history` | Yes | 子会话完整可见消息历史 |
| `phase` | Yes | 当前阶段或等价执行阶段信息 |
| `compaction_refs` | Context-dependent | 可由 projector 或 context pipeline 等价恢复 |
| `recovery_refs` | Context-dependent | 可由 projector 或 context pipeline 等价恢复 |
| `source_event_range` | Yes | 标识本次恢复基于哪些 durable 事件重建 |

**Validation**

- resume 成功前必须完成 `ResumeVisibleState` 的重建校验。
- 若 `message_history` 或 `phase` 无法可靠恢复，resume 必须失败而不是回退为空状态。

## 5. `InheritedContextBlock`（运行时 prompt 结构）

表示父传子的结构化继承背景。它属于 prompt 构建层，不属于 durable transcript。

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `block_kind` | `compact_summary \| recent_tail` | Yes | 继承块类型 |
| `content` | `String` | Yes | 注入 system block 的文本内容 |
| `token_budget` | `usize` | Yes | 本块预算 |
| `fingerprint_segment` | `String` | Yes | 对应 prompt cache 的局部指纹 |
| `source_revision` | `String` | Yes | 来自父上下文快照的版本/修订标识 |

**Validation**

- `InheritedContextBlock` 必须通过 `PromptDeclaration` 注入 system blocks，不得写成子消息流里的 `UserMessage`。
- `block_kind=recent_tail` 时必须经过确定性筛选和预算裁剪。

## 6. `PromptCacheScope`（派生结构）

表示 prompt 复用判断依赖的强指纹范围。它不是规格硬编码的 key 规则，而是测试和观测可以验证的输入域。

| Field | Type | Notes |
|-------|------|-------|
| `normalized_working_dir` | `String` | 规范化工作目录 |
| `active_profile` | `String` | 活动 profile |
| `tool_allowlist_fingerprint` | `String` | 工具集合指纹 |
| `prompt_rules_fingerprint` | `String` | `AGENTS.md` / prompt declarations / 相关规则指纹 |
| `skills_fingerprint` | `String` | skills 内容指纹 |
| `tool_metadata_fingerprint` | `String` | 工具元数据指纹 |
| `builder_version` | `String` | prompt builder / contributor 版本 |
| `fingerprint` | `String` | `runtime-prompt` 计算出的总指纹 |

**Validation**

- 任何会影响 prompt 构建结果的输入变化都必须改变 `fingerprint` 或等价强指纹。
- 不允许仅凭长度、条目数量或其他弱特征推断缓存可复用。

## 7. `PendingChildDelivery`（运行时桥接）

表示父智能体尚未消费的子交付。它只存在于运行时缓冲，不参与 durable 回放。

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `delivery_id` | `String` | Yes | 去重与消费追踪主键 |
| `parent_session_id` | `String` | Yes | 目标父会话 |
| `child_session_id` | `String` | Yes | 来源子会话 |
| `execution_id` | `String` | Yes | 来源执行实例 |
| `payload` | `serde_json::Value` | Yes | 一次性交付详情 |
| `state` | `queued \| waking_parent \| consumed` | Yes | 运行时状态 |
| `queued_at` | `DateTime<Utc>` | Yes | 排队时间 |

**Validation**

- `PendingChildDelivery` 必须与对应的 `ParentChildBoundaryFact(kind=delivered|completed|failed|cancelled)` 配对。
- 进程重启后可丢失 `PendingChildDelivery`，但不得因此丢失 durable 边界事实与子会话入口。

## 8. `ChildSummaryProjection`（读模型）

表示父视图和服务端状态接口消费的摘要投影。它从 durable 边界事实和 child session 元数据组合生成，不单独作为事实源。

| Field | Type | Notes |
|-------|------|-------|
| `child_session_id` | `String` | 子会话身份 |
| `latest_execution_id` | `String` | 最近执行实例 |
| `status` | `String` | 当前状态 |
| `status_source` | `durable \| live_overlay` | 状态来源 |
| `latest_summary` | `String` | 父视图摘要 |
| `open_session_id` | `String` | 直接打开子会话 |
| `latest_delivery_excerpt` | `Option<String>` | 最近交付摘要 |
| `diagnostic_code` | `Option<String>` | 失败或不一致时的诊断码 |

## 9. `LegacyHistoryRejection`（运行时/协议错误）

表示系统在遇到旧共享写入历史时返回的显式拒绝结果。它不是 durable 事实源，而是 cutover 后的稳定错误语义。

| Field | Type | Notes |
|-------|------|-------|
| `session_id` | `String` | 被拒绝的 legacy session |
| `error_code` | `unsupported_legacy_shared_history` | 稳定错误码 |
| `message` | `String` | 面向调用方的错误说明 |
| `required_action` | `upgrade_required \| cleanup_required` | 下一步动作 |

## Relationships

- 一个 `ChildSessionNode` 可以拥有多个 `ChildExecutionInstance`。
- 一个 `ChildExecutionInstance` 会生成零个或多个 `ParentChildBoundaryFact`。
- `ResumeVisibleState` 由 child session durable 历史重建，不单独持久化为新的事实源。
- `InheritedContextBlock` 与 `PromptCacheScope` 共同决定 prompt 层的继承与缓存行为。
- `PendingChildDelivery` 是 `ParentChildBoundaryFact` 与父 turn 继续处理之间的运行时桥接。
- `ChildSummaryProjection` 由 `ChildSessionNode` + `ParentChildBoundaryFact` + live overlay 组合产生。
- `LegacyHistoryRejection` 只在系统遇到不受支持的共享写入历史时返回，不参与 child session 领域建模。

## State Transitions

### `ChildExecutionInstance.status`

`pending -> running -> completed|failed|cancelled|terminated`

- `resume` 总是创建新的 `pending`/`running` 实例，不复用旧执行实例。
- `completed|failed|cancelled|terminated` 任一终态都必须生成父侧可读边界事实。

### `PendingChildDelivery.state`

`queued -> waking_parent -> consumed`

- 若进程在 `queued` 或 `waking_parent` 崩溃，运行时缓冲可消失，但父侧仍必须可通过 durable 边界事实追溯交付已经发生。
