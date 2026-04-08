# Data Model: 子 Agent Child Session 与协作工具重构

## 1. `ChildSessionNode`

表示一条 durable 的 parent/child agent 所有权节点。它回答“这个 child session 是谁、挂在哪个父 agent 下、现在处于什么生命周期”，不直接承载 transcript 内容。

| Field | Type | Required on new writes | Notes |
|-------|------|------------------------|-------|
| `agent_id` | `String` | Yes | 子 agent 的稳定身份；tool targeting 与 inbox 投递都以它为主键 |
| `session_id` | `String` | Yes | 子会话 id；用于直接加载 child transcript |
| `sub_run_id` | `String` | Yes | 与当前 subrun/status/history 兼容的稳定执行域 id |
| `parent_session_id` | `String` | Yes | 父会话 id |
| `parent_agent_id` | `Option<String>` | Yes | 顶层父 agent 为空；普通 child 必须存在 |
| `parent_turn_id` | `String` | Yes | 创建该 child session 的父 turn |
| `lineage_kind` | `spawn \| fork \| resume` | Yes | 区分普通创建、fork 继承、恢复继续 |
| `status` | `pending \| running \| completed \| failed \| aborted \| cancelled` | Yes | durable 生命周期状态 |
| `status_source` | `live \| durable \| legacyDurable` | Yes | 查询状态来源；读侧字段，不一定单独落盘 |
| `child_session_id` | `String` | Yes | 与 `session_id` 等值或兼容别名；用于 server/frontend DTO 平滑迁移 |
| `created_by_tool_call_id` | `Option<String>` | Yes | 触发创建的协作 tool call |

**Validation**

- `agent_id`、`session_id`、`sub_run_id` 在同一项目内必须稳定可重放
- `lineage_kind=resume` 只能引用已存在的 `agent_id`
- `parent_turn_id` 不能为空，即使父 turn 后续结束

## 2. `ChildSessionExecutionBoundary`

表示 child session 自身的执行边界快照。它只属于 child session，不从 parent storage mode 或 UI 路径反推。

| Field | Type | Notes |
|-------|------|-------|
| `run_mode` | `foreground \| background` | spawn/continue 的执行模式 |
| `storage_mode` | `sharedSession \| independentSession` | 当前仍保留，供兼容与读侧映射 |
| `approval_policy` | `inherit \| explicit(...)` | child session 自身审批边界 |
| `tool_scope` | `Vec<String>` / policy ref | 允许使用的工具范围 |
| `working_dir` | `PathBuf` | child 执行工作目录 |
| `isolation` | `none \| worktree \| snapshot` | 当前或未来隔离模式 |
| `profile_id` | `String` | child agent profile |
| `profile_context` | `serde_json::Value` | runtime policy 快照，用于恢复 |

## 3. `ChildAgentRef`

表示 tool 契约层和 UI/HTTP DTO 共享的稳定 child-agent 引用。它不是 live handle，而是 durable ref。

| Field | Type | Notes |
|-------|------|-------|
| `agent_id` | `String` | tool targeting 主键 |
| `session_id` | `String` | 打开子会话使用 |
| `sub_run_id` | `String` | 与现有 subrun/status surface 对齐 |
| `parent_agent_id` | `Option<String>` | ownership 关系 |
| `lineage_kind` | `spawn \| fork \| resume` | 创建来源 |
| `status` | `String` | 当前快照状态 |
| `openable` | `bool` | 是否可直接打开 child session |

## 4. `AgentInboxEnvelope`

表示一个面向目标 agent 的协作输入投递单元。runtime 通过它实现 tool 协作语义。

| Field | Type | Notes |
|-------|------|-------|
| `envelope_id` | `String` | durable 去重主键 |
| `target_agent_id` | `String` | 唯一消费方 |
| `source_agent_id` | `Option<String>` | 来源 agent；顶层用户输入可为空 |
| `source_session_id` | `String` | 来源会话 |
| `source_turn_id` | `Option<String>` | 来源 turn |
| `source_tool_call_id` | `Option<String>` | 哪个 tool 触发了投递 |
| `kind` | `instruction \| handoff \| wait_request \| close_request \| resume_request \| notification` | 投递类型 |
| `payload` | `serde_json::Value` | 结构化内容 |
| `delivery_status` | `queued \| delivered \| consumed \| superseded \| failed` | 投递生命周期 |
| `dedupe_key` | `String` | 恢复/重试去重键 |
| `created_at` | `DateTime<Utc>` | 便于排序与重放 |

**Validation**

- 同一 `dedupe_key` 对同一 `target_agent_id` 只能产生一次有效消费
- `notification` 类型不能反向要求目标 agent 直接修改其他 agent 状态

## 5. `CollaborationNotification`

表示 child session 对 parent session 的投影事件。它是父视图摘要的 durable 来源。

| Field | Type | Notes |
|-------|------|-------|
| `notification_id` | `String` | durable 通知 id |
| `parent_session_id` | `String` | 投影落入的父会话 |
| `parent_agent_id` | `Option<String>` | 目标父 agent |
| `child_ref` | `ChildAgentRef` | 子 agent 引用 |
| `kind` | `started \| progress_summary \| delivered \| waiting \| resumed \| closed \| failed` | 摘要种类 |
| `summary` | `String` | 父会话默认展示的文本 |
| `findings` | `Vec<String>` | 可选关键发现 |
| `artifacts` | `Vec<ArtifactRef>` | 会话入口或其他产物引用 |
| `final_reply_excerpt` | `Option<String>` | 最终回复的简短摘录 |
| `failure` | `Option<SubRunFailure>` | 失败投影 |

## 6. `ChildSessionViewProjection`

表示前端父/子双层视图使用的 read model。它不是 durable truth，只是服务端/前端的投影。

| Field | Type | Notes |
|-------|------|-------|
| `child_ref` | `ChildAgentRef` | 视图身份 |
| `title` | `String` | UI 标题 |
| `status` | `String` | 当前状态 |
| `summary_items` | `Vec<String>` | 父视图展示用摘要项 |
| `latest_tool_activity` | `Vec<String>` | 工具活动概览 |
| `has_final_reply` | `bool` | 是否已有最终回复 |
| `child_session_id` | `String` | 打开目标 child session |
| `has_descriptor_lineage` | `bool` | legacy 降级提示 |
| `active_path` | `Vec<String>` | 前端 breadcrumb/read model 使用 |

## 7. `ChildSessionLineageSnapshot`

表示 fork 或 resume 时需要保留的 lineage 快照。

| Field | Type | Notes |
|-------|------|-------|
| `snapshot_id` | `String` | lineage snapshot 标识 |
| `base_agent_id` | `String` | 来源 agent |
| `base_session_id` | `String` | 来源 session |
| `fork_mode` | `fullHistory \| lastNTurns(n)` | 未来 fork 的上下文模式 |
| `compacted_context_ref` | `Option<ArtifactRef>` | 若通过 compact summary 继承，记录引用 |
| `created_by` | `String` | 创建来源（spawn/fork/resume tool） |

## Relationships

- 一个 `ChildSessionNode` 必须拥有一个 `ChildSessionExecutionBoundary`。
- 一个 `ChildAgentRef` 必须能回指到一个 `ChildSessionNode`。
- 一个 `AgentInboxEnvelope` 只能被一个 `target_agent_id` 消费，但可投影为多个 read model。
- 一个 `CollaborationNotification` 必须引用一个 `ChildAgentRef`，并落到一个父会话中。
- `ChildSessionViewProjection` 由 `CollaborationNotification` + child session history 组合生成，不单独落盘为事实源。
- `ChildSessionLineageSnapshot` 只在 `lineage_kind=fork` 或 `resume` 时存在。
