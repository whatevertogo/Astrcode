# Session Fork

会话 fork 能力：允许从源 session 的稳定前缀创建独立的新 session，继承事件历史后独立发展。

## Purpose

提供从已有 session 的某个稳定时间点（已完成 turn）创建分支会话的能力。fork 后的新 session 继承 fork 点之前的完整事件历史，并独立执行后续 turn。主要用于探索不同对话方向而不影响原始会话。

## Requirements

### Requirement: 稳定前缀 fork

系统 SHALL 允许从源 session 的稳定前缀 fork 出独立的新 session。稳定前缀是指截至某个已完成 turn（以 `TurnDone` 事件为标记）的连续事件序列。fork 后的新 session 继承该点之前的完整事件历史，之后独立发展。

fork 产生的新 session SHALL 在 `SessionStart` 事件中记录 `parent_session_id`（源 session ID）和 `parent_storage_seq`（fork 点的 storage_seq）。

fork 产生的新 session SHALL 通过现有 `SessionBranched` catalog event 通知前端。

fork 后的新 session SHALL 拥有与源 session 完全相同的 prompt prefix（不改 system prompt、不改工具集），确保 LLM KV cache 前缀命中。

#### Scenario: 从已完成 turn 的 turnId fork

- **WHEN** 调用 `fork_session` 传入已完成 turn 的 `turn_id`
- **THEN** 系统创建新 session，复制源 session 从第一个事件到该 turn 的 `TurnDone` 事件之间的所有事件（跳过源 `SessionStart`），新 session 投影后 `phase = Idle`

#### Scenario: 尾部 fork 且源 session 空闲

- **WHEN** 调用 `fork_session` 传入 `ForkPoint::Latest`，且源 session 当前无活跃 turn
- **THEN** 系统复制源 session 的全部事件历史到新 session

#### Scenario: 尾部 fork 且源 session 正忙

- **WHEN** 调用 `fork_session` 传入 `ForkPoint::Latest`，且源 session 当前有活跃 turn
- **THEN** 系统只复制到最后一个 `TurnDone` 事件之后的最后一个事件，不复制活跃 turn 的半截事件

#### Scenario: 新 session 可立即接收 prompt

- **WHEN** fork 成功完成
- **THEN** 新 session 的投影状态 `phase = Idle`，可以立即接收新 prompt 并执行 turn

#### Scenario: 谱系字段正确记录

- **WHEN** fork 成功完成
- **THEN** 新 session 的 `SessionStart` 事件包含 `parent_session_id`（源 session ID）和 `parent_storage_seq`（fork 点 seq），且查询 session 元信息时 `parentSessionId` 指向源 session

### Requirement: fork 点校验

系统 SHALL 校验 fork 点落在稳定前缀内。以下情况 SHALL 返回错误：

- fork 点对应的 turn 尚未完成（无 `TurnDone` 事件）→ `Validation` 错误
- 传入的 `turn_id` 不存在 → `NotFound` 错误
- 传入的 `storage_seq` 超出源 session 事件范围 → `Validation` 错误
- 传入的 `storage_seq` 落在活跃 turn 内 → `Validation` 错误
- `turn_id` 和 `storage_seq` 同时传入 → `Validation` 错误

#### Scenario: 未完成 turn 的 fork 点被拒绝

- **WHEN** 调用 `fork_session` 传入一个尚未完成的 turn 的 `turn_id`（该 turn 没有 `TurnDone` 事件）
- **THEN** 返回 `Validation` 错误，消息说明该 turn 尚未完成，不能作为 fork 点

#### Scenario: 活跃 turn 内的 storage_seq 被拒绝

- **WHEN** 调用 `fork_session` 传入的 `storage_seq` 落在活跃 turn 的事件范围内（不在稳定前缀中）
- **THEN** 返回 `Validation` 错误，消息说明该点位于未完成的 turn 内

#### Scenario: 不存在的 turn_id 被拒绝

- **WHEN** 调用 `fork_session` 传入不存在的 `turn_id`
- **THEN** 返回 `NotFound` 错误，消息包含不存在的 `turn_id`

#### Scenario: 超出范围的 storage_seq 被拒绝

- **WHEN** 调用 `fork_session` 传入超出源 session 事件范围的 `storage_seq`
- **THEN** 返回 `Validation` 错误，消息说明 seq 范围不合法

#### Scenario: 同时传入 turnId 和 storageSeq 被拒绝

- **WHEN** `POST /api/sessions/:id/fork` 请求体同时包含 `turnId` 和 `storageSeq`
- **THEN** 返回 `400 Validation` 错误，消息说明两者互斥

### Requirement: fork 点解析

系统 SHALL 支持三种 fork 点标识方式：

| 输入 | 解析规则 |
|------|---------|
| 只传 `storage_seq` | 直接使用，校验必须落在稳定前缀内 |
| 只传 `turn_id` | 找到该 turn 的 `TurnDone` 事件的 `storage_seq` |
| 都不传 | fork 到源 session 的稳定前缀末尾 |

#### Scenario: 通过 turn_id 解析 fork 点

- **WHEN** 调用 `fork_session` 传入 `ForkPoint::TurnEnd(turn_id)`，且该 turn 已完成
- **THEN** 系统找到该 `turn_id` 对应的 `TurnDone` 事件的 `storage_seq`，以此作为 fork 点

#### Scenario: 通过 storage_seq 解析 fork 点

- **WHEN** 调用 `fork_session` 传入 `ForkPoint::StorageSeq(seq)`，且 seq 在稳定前缀内
- **THEN** 系统直接使用该 `seq` 作为 fork 点

#### Scenario: 默认尾部 fork

- **WHEN** HTTP 请求 `POST /api/sessions/:id/fork` 请求体为空 `{}`
- **THEN** 系统解析为 `ForkPoint::Latest`，fork 到源 session 的稳定前缀末尾

### Requirement: 客户端消息级快捷入口

系统 SHALL 允许客户端在历史消息上提供"从此处 fork"便捷入口，但该入口只是一层 UI 映射，不能扩展服务端 fork 点类型。

客户端在消息级发起 fork 时 SHALL 先将该消息解析为所属已完成 turn 的 `turnId`，再调用现有 `POST /api/sessions/:id/fork` API。服务端 SHALL 不接受 `messageId` 作为 fork 点。

#### Scenario: 历史消息映射到 turnId

- **WHEN** 用户在一条带有 `turnId` 的历史消息上触发"从此处 fork"
- **THEN** 客户端读取该消息的 `turnId`，并调用 `POST /api/sessions/:id/fork { turnId }`

#### Scenario: 不可稳定映射的消息不显示 fork

- **WHEN** 某条消息没有 `turnId`、属于活跃 turn、或不是稳定 root turn 消息
- **THEN** 客户端不显示消息级 fork 入口

### Requirement: 事件复制规则

系统 SHALL 复制源 session 从第一个事件到 fork 点的所有事件到新 session，但：
- 不复制源 session 的 `SessionStart` 事件（新 session 有自己的 `SessionStart`）
- 复制所有其他事件类型（UserMessage、AssistantDelta、ToolCall、ToolResult、CompactApplied、TurnDone 等）
- 事件的 `storage_seq` 不保留，由新 session 的 EventLog 重新分配

#### Scenario: compact 事件正常复制

- **WHEN** 源 session 在 fork 点之前有 `CompactApplied` 事件
- **THEN** `CompactApplied` 事件正常复制到新 session，新 session 继承压缩后的上下文视图

### Requirement: HTTP API

系统 SHALL 提供 `POST /api/sessions/:id/fork` 端点。

请求体：
```json
{
  "turnId": "turn-abc123",
  "storageSeq": 42
}
```

两个字段均为可选，互斥。成功响应为 `SessionListItem`（新 session 的元信息，`parentSessionId` 指向源 session）。

#### Scenario: 源 session 不存在

- **WHEN** `POST /api/sessions/:id/fork` 中 `:id` 对应的 session 不存在
- **THEN** 返回 `404 NotFound` 错误

#### Scenario: 成功 fork 返回新 session 元信息

- **WHEN** `POST /api/sessions/:id/fork` 请求合法且 fork 成功
- **THEN** 返回 `SessionListItem`，其中 `parentSessionId` 指向源 session，前端收到 `SessionBranched` catalog event

#### Scenario: fork 成功后立即进入新 session

- **WHEN** 前端收到成功的 fork 响应
- **THEN** 前端立即切换到返回的新 session，并加载该 session 的 conversation snapshot

### Requirement: 后台调用契约

`SessionRuntime` SHALL 提供 `fork_session(source_session_id, fork_point) -> Result<ForkResult>` 方法。`fork_point` 为 runtime 内部枚举 `StorageSeq(u64) | TurnEnd(String) | Latest`。返回 `ForkResult { new_session_id, fork_point_storage_seq, events_copied }`。不触发任何 turn 执行。

`application` SHALL 提供 `fork_session(session_id, selector) -> Result<SessionMeta>` use case，其中 `selector` MUST 为 application-owned fork selector，而不是 runtime `ForkPoint`。`AppSessionPort` 的实现 SHALL 在 port 边界内部把该 selector 映射为 runtime `ForkPoint`。

#### Scenario: 后台通过 SessionRuntime fork

- **WHEN** 后台流程调用 `SessionRuntime::fork_session`
- **THEN** 返回 `ForkResult` 包含新 session ID、fork 点 storage_seq 和复制的事件数量，不触发 turn 执行

#### Scenario: server 通过 application-owned selector 发起 fork

- **WHEN** `server` 需要从 HTTP 请求触发 session fork
- **THEN** 它 SHALL 通过 `application` 定义的 fork selector 调用 `App::fork_session`
- **AND** SHALL NOT 直接构造 runtime `ForkPoint`

#### Scenario: runtime fork enum 不再穿透到 application 边界

- **WHEN** 检查 `server -> application` 的 fork 调用合同
- **THEN** 对外暴露的类型 SHALL 是 application-owned selector
- **AND** runtime `ForkPoint` SHALL 只留在 application port 实现与 session-runtime 内部

#### Scenario: server 只收到 fork 后的 SessionMeta

- **WHEN** `server` 通过 `application` 发起 fork
- **THEN** `App::fork_session()` SHALL 返回 `SessionMeta`
- **AND** runtime `ForkResult` SHALL 只留在 application / port 内部
