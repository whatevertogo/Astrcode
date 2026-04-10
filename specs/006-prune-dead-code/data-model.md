# Data Model: 删除死代码与冗余契约收口

本特性不是新增业务能力，而是把当前已经存在但重复、错位或只为兼容存在的模型收口成唯一正式表达。

## 1. `SupportSurfaceDecision`

表示一个 candidate surface 的最终归类。

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `surface_id` | `String` | Yes | 唯一标识 |
| `layer` | `frontend \| protocol \| server \| runtime \| core \| docs \| tests` | Yes | 所属边界 |
| `decision` | `retain \| migrate_then_remove \| remove` | Yes | 最终动作 |
| `owner_boundary` | `String` | No | 保留项必须可说明 owner |
| `active_consumer` | `Option<String>` | No | 阻止立即删除的主线消费者 |
| `replacement` | `Option<String>` | No | 若需迁移，说明唯一替代入口 |
| `verification_rule` | `String` | Yes | 如何证明动作落地 |

**Validation**

- `retain` 必须同时有 `owner_boundary` 与 `active_consumer`。
- `migrate_then_remove` 必须有 `replacement`。
- `tests` / `docs` 不能独立构成 `retain` 理由。

## 2. `AgentStatus`（canonical subrun status）

本特性的唯一 subrun / child-agent 状态模型。

| Variant |
|---------|
| `Pending` |
| `Running` |
| `Completed` |
| `Cancelled` |
| `Failed` |
| `TokenExceeded` |

**Rules**

- `Completed`、`Cancelled`、`Failed`、`TokenExceeded` 都是终态。
- 不再允许 `SubRunOutcome` 这类平行状态模型继续存在。
- protocol 可保留 DTO-only 镜像枚举，但语义和值域必须与此模型一一对应。

## 3. `ExecutionAccepted`

统一 prompt submit / root execute 的稳定回执。

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `session_id` | `String` | Yes | 被接收的执行所在会话 |
| `turn_id` | `String` | Yes | 当前 turn 标识 |
| `agent_id` | `Option<String>` | No | 仅 root execute 等有独立 agent 时存在 |
| `branched_from_session_id` | `Option<String>` | No | 仅 prompt submit 分支场景存在 |

**Validation**

- `session_id` / `turn_id` 必填。
- `agent_id` 与 `branched_from_session_id` 可以同时为空，但不能引入第二套 receipt 类型替代本模型。

## 4. `SubRunHandle`（canonical lineage + live state）

`SubRunHandle` 是 subrun 运行时句柄与 lineage 核心事实的唯一 owner。

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `sub_run_id` | `String` | Yes | 稳定子执行域 ID |
| `agent_id` | `String` | Yes | 运行时 agent 实例 ID |
| `session_id` | `String` | Yes | 事件写入所在 session |
| `child_session_id` | `Option<String>` | No | 独立子会话时存在 |
| `depth` | `usize` | Yes | 父子树深度 |
| `parent_turn_id` | `String` | Yes | 本次收口后改为必填 |
| `parent_agent_id` | `Option<String>` | No | 父 agent，可为空 |
| `agent_profile` | `String` | Yes | profile 标识 |
| `storage_mode` | `SubRunStorageMode` | Yes | 仅表示写入位置 |
| `status` | `AgentStatus` | Yes | 当前状态 |

**Validation**

- `parent_turn_id` 既然是 lineage 核心事实，就不能再为 downgrade 保持 optional。
- 删除 `descriptor()`；调用方直接从 `SubRunHandle` 取值。
- storage mode 不能改变 ownership 解释。

## 5. `AgentEventContext`

`AgentEventContext` 继续作为事件侧上下文投影，但 subrun 场景改为直接从 `SubRunHandle` 构造。

| Field | Source |
|-------|--------|
| `agent_id` | `SubRunHandle.agent_id` |
| `parent_turn_id` | `SubRunHandle.parent_turn_id` |
| `agent_profile` | `SubRunHandle.agent_profile` |
| `sub_run_id` | `SubRunHandle.sub_run_id` |
| `invocation_kind` | 固定为 `SubRun` |
| `storage_mode` | `SubRunHandle.storage_mode` |
| `child_session_id` | `SubRunHandle.child_session_id` |

**Validation**

- subrun handle -> event context 使用统一 `From<&SubRunHandle>` 实现。
- `sub_run()` 工厂方法保留给没有 handle 的非标准构造场景。

## 6. `ChildAgentRef`（canonical child reference）

`ChildAgentRef` 收口为 child identity + lineage + status + 唯一 open target 的正式事实。

| Field | Type | Required |
|-------|------|----------|
| `agent_id` | `String` | Yes |
| `session_id` | `String` | Yes |
| `sub_run_id` | `String` | Yes |
| `parent_agent_id` | `Option<String>` | No |
| `lineage_kind` | `ChildSessionLineageKind` | Yes |
| `status` | `AgentStatus` | Yes |
| `open_session_id` | `String` | Yes |

**Removed Fields**

- `openable`

**Validation**

- `open_session_id` 是唯一 canonical child open target；通知、DTO 与其他外层载荷不得重复持有同值字段。
- 是否可打开由是否存在 canonical open target 决定，不再由 duplicated bool 决定。
- `ChildSessionNode::child_ref()` 只能返回正式 child 事实，不能顺手注入额外 UI 派生值。

## 7. `ChildSessionNotification`

父侧可消费的 child 通知事实。

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `notification_id` | `String` | Yes | 通知 ID |
| `child_ref` | `ChildAgentRef` | Yes | 含 canonical open target |
| `kind` | `ChildSessionNotificationKind` | Yes | 通知类型 |
| `summary` | `String` | Yes | 父侧摘要 |
| `status` | `AgentStatus` | Yes | 当前状态 |
| `source_tool_call_id` | `Option<String>` | No | 来源工具调用 |
| `final_reply_excerpt` | `Option<String>` | No | 终态摘录 |

**Validation**

- 外层 `open_session_id` 删除；调用方一律从 `child_ref.open_session_id` 读取。
- protocol DTO 只允许投影该嵌套字段，不得重新复制一份外层字段。

## 8. `ProtocolAgentStatusDto`

protocol 层 child/subrun 状态的强类型镜像枚举。

| Variant |
|---------|
| `Pending` |
| `Running` |
| `Completed` |
| `Cancelled` |
| `Failed` |
| `TokenExceeded` |

**Validation**

- protocol 不允许继续把 child/subrun 状态表达为 `String`。
- DTO 枚举值域必须与 canonical `AgentStatus` 一一对应。

## 9. `PromptMetricsPayload`

被 storage event、agent event 和 protocol event 共同复用的共享指标载荷。

| Field | Type | Required |
|-------|------|----------|
| `step_index` | `u32` | Yes |
| `estimated_tokens` | `u32` | Yes |
| `context_window` | `u32` | Yes |
| `effective_window` | `u32` | Yes |
| `threshold_tokens` | `u32` | Yes |
| `truncated_tool_results` | `u32` | Yes |
| `provider_input_tokens` | `Option<u32>` | No |
| `provider_output_tokens` | `Option<u32>` | No |
| `cache_creation_input_tokens` | `Option<u32>` | No |
| `cache_read_input_tokens` | `Option<u32>` | No |
| `provider_cache_metrics_supported` | `bool` | Yes |
| `prompt_cache_reuse_hits` | `u32` | Yes |
| `prompt_cache_reuse_misses` | `u32` | Yes |

**Validation**

- 三层事件只能复用该 payload，不再各自维护一份完整字段清单。
- 共享 payload 不改变各层 envelope/metadata，只收口重复字段定义。

## 10. `CompactionReasonMapping`

内部 compaction reason 到 durable trigger 的唯一归一规则。

| Internal Reason | Durable Trigger |
|-----------------|-----------------|
| `Auto` | `Auto` |
| `Reactive` | `Auto` |
| `Manual` | `Manual` |

**Validation**

- `Reactive` 保留为 runtime / hook 内部原因，不扩展 durable trigger 集合。
- 该映射必须集中定义，不允许多处手写并漂移。

## 11. `ChildNavigationTarget`（view-layer projection）

这是 child navigation 需要的唯一打开目标，不属于 core 领域模型。

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `target_session_id` | `String` | Yes | UI 应打开的 session |
| `source` | `notification \| history_message \| durable_child_node` | Yes | 来源 |

**Validation**

- `openable` 不再单独存在；UI 以 target 是否存在判断能否打开。
- `target_session_id` 通常直接取自 `child_ref.open_session_id`；该 projection 不能再引入第二份真相字段。
- summary projection route 不是该 target 的合法来源。

## 12. `LegacyInputFailure`

表示不再支持的旧输入在主线中的明确失败。

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `failure_code` | `String` | Yes | 稳定错误码或等价错误分类 |
| `reason` | `String` | Yes | 失败原因 |
| `surface` | `history \| subrun \| control` | Yes | 失败发生位置 |

**Validation**

- 旧输入可以失败，但不能再产出 downgrade tree、legacyDurable source 或伪造 lineage。

## Canonicalization Map

| Removed / Reduced Model | Canonical Target |
|-------------------------|------------------|
| `SubRunOutcome` | `AgentStatus` |
| `SubRunDescriptor` | `SubRunHandle` 直接字段 |
| optional `parent_turn_id` | required `SubRunHandle.parent_turn_id` |
| `PromptAccepted` + `RootExecutionAccepted` + runtime duplicate | `ExecutionAccepted` |
| 手工 `AgentEventContext::sub_run(...)` 字段拼装 | `From<&SubRunHandle>` |
| `ChildAgentRef.openable` | 删除，由 canonical open target 判断 |
| `ChildSessionNotification.open_session_id` | `ChildAgentRef.open_session_id` |
| protocol `status: String` | `ProtocolAgentStatusDto` |
| 三层 `PromptMetrics` variant 字段清单 | `PromptMetricsPayload` |
| 散落的 compaction reason 手写映射 | `CompactionReasonMapping` |
| `legacyDurable` / descriptorless downgrade view | `LegacyInputFailure` |

## Relationships

- 一个 `SupportSurfaceDecision` 可以指向一个 canonical model 收口动作。
- `SubRunHandle` 是 `AgentEventContext` 与 `ChildNavigationTarget` 的上游事实来源之一。
- `ChildAgentRef` 表达 child identity，并承载唯一 canonical open target。
- `ChildSessionNotification` 通过嵌套 `child_ref` 暴露 open target，不再双写。
- `PromptMetricsPayload` 被 storage/domain/protocol 三层事件引用。
- `LegacyInputFailure` 取代 legacy downgrade public model。

## State Transitions

### Support Surface

`retain` 保留并更新 live docs/tests。  
`migrate_then_remove` 必须先切换调用方，再删除旧 surface。  
`remove` 直接从代码、协议、文档和测试中退出。

### Subrun Status

`Pending -> Running -> Completed | Cancelled | Failed | TokenExceeded`

- `TokenExceeded` 与其他终态地位相同。
- 不再允许 `Aborted -> Cancelled` 这种跨模型映射层继续存在。
