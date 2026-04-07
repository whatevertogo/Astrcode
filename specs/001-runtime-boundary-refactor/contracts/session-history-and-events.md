# Contract: Session History And Events

## Scope

该契约定义两条历史消费主线的统一语义：

- `GET /api/sessions/{id}/history`
- `GET /api/sessions/{id}/events`

目标不是新增第二套 view model，而是确保这两条路径都以同一份 durable lineage 事实解释 subrun 所属关系、scope 过滤和 legacy 降级行为。

## Query Parameters

| Name | Type | Required | Meaning |
|------|------|----------|---------|
| `subRunId` | `string` | No | 指定过滤目标 subrun |
| `scope` | `self \| directChildren \| subtree` | No | 过滤范围；缺省为 `subtree`，但只有在提供 `subRunId` 时才允许 |
| `afterEventId` | `string` | `events` only | SSE catch-up 光标 |

## Shared Rules

1. `/history` 与 `/events` 必须使用同一套 `SessionEventFilterSpec` 语义。
2. 两者都必须从同一份 `ExecutionLineageIndex` 推导 parent/child 关系。
3. 对同一 session、同一 `subRunId + scope`，`/history` 首次快照与 `/events` 的 initial replay 结果必须一致。
4. `self` 仅依赖事件上的 `subRunId`，不依赖 ancestry index。
5. `directChildren` / `subtree` 必须依赖 durable lineage descriptor，禁止从事件顺序、turn owner 或 UI tree 反推。

## Lifecycle Event Payload Contract

`subRunStarted` 与 `subRunFinished` 必须携带相同的 lineage 主键与 trigger 关联。

```json
{
  "event": "subRunStarted",
  "turnId": "turn-parent-1",
  "agent": {
    "agentId": "agent-child-1",
    "agentProfile": "reviewer",
    "subRunId": "subrun-1",
    "storageMode": "sharedSession",
    "childSessionId": null
  },
  "descriptor": {
    "subRunId": "subrun-1",
    "parentTurnId": "turn-parent-1",
    "parentAgentId": "agent-root-1",
    "depth": 1
  },
  "toolCallId": "call_spawn_001",
  "resolvedOverrides": {
    "storageMode": "sharedSession",
    "inheritWorkingDir": true
  },
  "resolvedLimits": {
    "maxSteps": 128
  }
}
```

```json
{
  "event": "subRunFinished",
  "turnId": "turn-parent-1",
  "agent": {
    "agentId": "agent-child-1",
    "agentProfile": "reviewer",
    "subRunId": "subrun-1",
    "storageMode": "sharedSession",
    "childSessionId": null
  },
  "descriptor": {
    "subRunId": "subrun-1",
    "parentTurnId": "turn-parent-1",
    "parentAgentId": "agent-root-1",
    "depth": 1
  },
  "toolCallId": "call_spawn_001",
  "result": {
    "status": "completed"
  },
  "stepCount": 18,
  "estimatedTokens": 3120
}
```

## Scope Semantics

### `scope=self`

- 返回目标 subrun 自身的生命周期事件、普通消息事件、工具事件、error/turnDone 等所有 `subRunId == target` 的事件。
- 不包含任何子执行事件。
- legacy 历史仍然允许使用。

### `scope=directChildren`

- 只返回 `descriptor.parentAgentId` 对应 target agent 的那些直接子执行事件。
- target 自身事件不包含在内，除非另有显式参数扩展；本次设计保持“仅直接子级”。
- 若 lineage index 发现 target 或候选 child 中存在缺失 `descriptor` 的 legacy gap，接口返回显式错误，不做猜测。

### `scope=subtree`

- 返回 target 自身 + 所有递归后代的事件。
- ancestry 仅通过 `descriptor.parentAgentId` 建图。
- 若构树所需 lineage 缺失，接口返回显式错误，不做 partial subtree 猜测。

## Legacy Behavior

| Situation | `/history` / `/events` behavior |
|-----------|---------------------------------|
| 生命周期事件缺少 `descriptor` | lifecycle payload 中 `descriptor = null` |
| 生命周期事件缺少 `toolCallId` | `toolCallId = null` |
| `scope=self` | 仍允许 |
| `scope=directChildren` / `scope=subtree` | 返回 `409 lineage metadata unavailable for requested scope` |

## SSE Catch-up Rules

- `last-event-id` / `afterEventId` 只决定增量起点，不得改变 lineage 语义。
- filtered SSE 的 initial replay 与 lag recovery 都必须复用同一个 lineage index 构建逻辑。
- lag recovery 期间若发现 lineage 需要的 durable 事实缺失，必须发显式错误事件或直接失败，而不是回退到启发式过滤。

## Implementation Checklist

- [x] `/history` 与 `/events` 的 shared rules 明确要求同一 `SessionEventFilterSpec` 与 `ExecutionLineageIndex`。
- [x] `scope=self/directChildren/subtree` 的行为和拒绝条件已显式定义。
- [x] legacy 行为中 `scope=directChildren/subtree -> 409` 的约束已写入契约。
- [x] lifecycle payload 示例与 `design-subrun-protocol.md` 的字段集合保持一致。

