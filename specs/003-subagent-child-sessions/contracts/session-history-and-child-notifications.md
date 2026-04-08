# Contract: Session History And Child Notifications

## 目的

定义 parent session、child session、server DTO 与 frontend read model 之间的投影契约。

## Parent Session Contract

parent session history/events 中只允许出现可消费的 child 投影：

- `child_started`
- `child_progress_summary`
- `child_delivered`
- `child_waiting`
- `child_resumed`
- `child_closed`
- `child_failed`

每条投影至少包含：

- `childRef`
- `summary`
- `status`
- `openSessionId`
- `sourceToolCallId`

### Parent Session Must Not Contain

- child session 的完整 transcript
- child 原始 progress event 序列
- raw JSON payload
- 内部 inbox envelope 明文

## Child Session Contract

child session 的 history/events 保留完整 transcript：

- thinking
- tool activity
- assistant replies
- 由 parent 投递过来的协作输入

child session 必须可以通过标准 session history/events 入口直接加载，而不是要求调用方从 parent history 里重新过滤。

## Status Contract

`child status` 投影至少需要提供：

- `agentId`
- `sessionId`
- `subRunId`
- `lineageKind`
- `status`
- `statusSource`
- `parentAgentId`
- `hasDescriptorLineage`

若为 legacy 历史，必须显式标注 `statusSource=legacyDurable` 或等价降级语义。

## Frontend Read Model Contract

前端默认应使用两层 read model：

1. `ParentChildSummaryList`
2. `ActiveChildSessionView`

前端不得把以下内容当成 durable truth：

- breadcrumb
- activeSubRunPath
- collapsed/expanded UI state
- mixed-session thread reconstruction

## Compatibility

旧的 subrun-only surface 可以在迁移期内继续读，但必须满足：

- 新 child session 模型优先
- legacy 路径明确标注能力受限
- 不允许继续伪造完整 parent/child 关系
