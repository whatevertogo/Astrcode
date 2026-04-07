# Contract: Execution Status And Agent Resolution

## Scope

该契约覆盖三类执行相关入口：

- `POST /api/v1/agents/{id}/execute`
- `GET /api/v1/sessions/{id}/subruns/{sub_run_id}`
- `POST /api/v1/sessions/{id}/subruns/{sub_run_id}/cancel`

它们必须共享同一份 execution resolution 与 durable/live status 语义。

## Root Agent Execution

### Request

```json
{
  "task": "review the current backend runtime design",
  "context": [],
  "maxSteps": 64,
  "contextOverrides": {
    "storageMode": "sharedSession"
  },
  "workingDir": "D:/repo/project-a"
}
```

### Rules

1. `workingDir` 为必填，缺失时返回 `400`。
2. resolver 必须使用 `workingDir` 构建 agent profile snapshot；禁止静默退回 `std::env::current_dir()`。
3. 返回的 `sessionId` / `turnId` / `agentId` 只说明执行已接受，不说明 lineage 已完成。
4. 同一个 `workingDir` 下的 agent hot reload 必须只影响该 resolver scope，不影响其他工作目录的 execution context。

### Response

```json
{
  "accepted": true,
  "message": "agent 'reviewer' execution accepted; subscribe to /api/sessions/session-1/events for progress",
  "sessionId": "session-1",
  "turnId": "turn-1",
  "agentId": "agent-root-1"
}
```

## SubRun Status

### Response Shape

```json
{
  "subRunId": "subrun-1",
  "descriptor": {
    "subRunId": "subrun-1",
    "parentTurnId": "turn-parent-1",
    "parentAgentId": "agent-root-1",
    "depth": 1
  },
  "toolCallId": "call_spawn_001",
  "source": "live",
  "agent": {
    "agentId": "agent-child-1",
    "agentProfile": "reviewer",
    "sessionId": "session-1",
    "childSessionId": null,
    "storageMode": "sharedSession"
  },
  "status": "running",
  "result": null,
  "stepCount": 12,
  "estimatedTokens": 1830,
  "resolvedOverrides": {
    "storageMode": "sharedSession"
  },
  "resolvedLimits": {
    "maxSteps": 64
  }
}
```

### `source` semantics

| Value | Meaning |
|-------|---------|
| `live` | 当前来自 `runtime-agent-control` 的运行态 handle，并叠加 durable descriptor |
| `durable` | live handle 不存在，但 durable lifecycle event 足够重建 status |
| `legacyDurable` | 只读到了 legacy 历史，status 可用，但 lineage / tool call 信息不完整 |

### Status Rules

1. `descriptor` 与 `toolCallId` 是 durable facts；`status`、`stepCount`、`estimatedTokens` 允许被 live overlay 更新。
2. 当 `source=legacyDurable` 时，`descriptor` 与 `toolCallId` 可以为空，服务端不得伪造默认 lineage。
3. 重启进程后，同一个已完成 subrun 的 `source` 可以从 `live` 变成 `durable`，但 `descriptor` 与 `result` 语义必须保持一致。

## Cancel SubRun

### Success and Failure

| Case | Response |
|------|----------|
| live running subrun belongs to session | `204 No Content` |
| subrun exists but已进入终态 | `409 Conflict` |
| subrun 只存在于 legacy durable 历史且无法 live cancel | `409 Conflict` |
| session 或 subrun 不存在 | `404 Not Found` |

### Rules

1. cancel 只能作用于当前 session 所拥有的 subrun。
2. durable status 只能证明“它存在过”，不能直接替代 live cancel handle。
3. 若请求命中了错误 session 或 lineage 缺失导致归属无法确认，必须返回显式错误。

## Frontend Consumption Rules

1. frontend 不再从 `parentTurnId -> turn owner` 启发式推导 parent/child，而是以 `descriptor.parentAgentId` + lifecycle 索引为准。
2. 执行列表、subrun tree、detail panel 与 cancel button 都必须消费同一个 status shape。
3. `source=legacyDurable` 时，UI 可以展示“lineage 不完整”，但不能伪造完整 breadcrumb 或 subtree。

