## Context

AstrCode 的 EventStore 基于 JSONL 事件日志，每个事件携带单调递增的 `storage_seq`。`SessionStart` 事件已有 `parent_session_id` / `parent_storage_seq` 谱系字段。现有 `branch_session_from_busy_turn`（`crates/session-runtime/src/turn/branch.rs`）已经实现了"复制源 session 事件到新 session"的核心逻辑，但被硬编码为"session 正忙时自动分叉"这一个场景。

现有自动 branch 的关键设计决策是**只复制稳定前缀**——`stable_events_before_active_turn()` 通过 `turn_id()` 匹配找到活跃 turn 的第一个事件位置，截取它之前的所有事件。这确保新 session 的投影状态 `phase = Idle`（因为缺少 `TurnDone` 不会让 projector 回到 Idle）。通用 fork 必须继承这个约束。

本 change 将这个核心逻辑泛化为通用的 session fork 能力。

## Goals / Non-Goals

**Goals:**

- 支持从源 session 的稳定前缀 fork 出独立的新 session
- 前端可通过 `turnId` 指定 fork 点（必须是已完成的 turn）
- 前端可在历史消息上提供"从此处 fork"便捷入口，但最终必须映射到所属已完成 turn 的 `turnId`
- 后台流程可通过 `storageSeq` 精确指定 fork 点（必须落在稳定前缀内）
- fork 后的新 session 拥有完全相同的 prompt prefix，确保 KV cache 前缀命中
- 新 session 投影后 `phase = Idle`，可立即接收新 prompt
- 新 session 记录谱系关系（`parent_session_id` + `parent_storage_seq`）
- fork 成功后前端立即切换到新 session，形成明确的分支工作流

**Non-Goals:**

- 不做 fork 合并（merge back）
- 不做 fork 时修改 prompt / 工具集 / 任何执行参数
- 不暴露给 LLM 作为工具调用
- 不替换现有的自动 branch 逻辑
- 不改 `EventStore` trait
- 不做 partial fork（fork 到活跃 turn 的中间事件）
- 不在 runtime / protocol 中引入 `messageId` 作为新的 fork 真相点位

## Decisions

### D1: fork 只接受稳定前缀

fork 点必须落在已完成的 turn 边界上。

理由：`AgentStateProjector`（`crates/core/src/projection/agent_state.rs`）通过事件流投影状态。当遇到 `UserMessage(origin=User)` 时 `phase → Thinking`，遇到 `TurnDone` 时 `phase → Idle`。如果 fork 点落在活跃 turn 中间，投影结果会缺少 `TurnDone` 事件，`phase` 停留在 `Thinking`，新 session 无法接收 prompt。

现有自动 branch 的 `stable_events_before_active_turn`（`branch.rs:151`）正是出于同样考虑——用 `turn_id()` 匹配找到活跃 turn 起始位置，截取之前的事件。

稳定前缀的精确定义：
- 事件序列中，截至某个 `TurnDone` 事件（含）的所有事件
- `TurnEnd(turn_id)` fork 点：找到该 turn_id 的 `TurnDone` 事件，截取到该位置
- `StorageSeq(seq)` fork 点：seq 对应的事件必须属于某个已有 `TurnDone` 的 turn，否则不在稳定前缀内
- `Latest`：源 session 当前 `phase = Idle` 时取全部事件；`phase = Thinking` 时取到最后一个 `TurnDone` 之后的事件

### D2: fork 点标识支持 `storageSeq` 和 `turnId` 双入口

前端按 turn 展示对话，用户点"从此处 fork"一定是在某个 turn 上，因此 `turnId` 是最自然的入口。后台流程（如 compact）可能需要更精确的 `storageSeq`。

后端解析规则：
- 只传 `storageSeq`：直接使用，校验落在稳定前缀内
- 只传 `turnId`：找到该 turn 的 `TurnDone` 事件的 `storage_seq`
- 都不传：fork 到源 session 的稳定前缀末尾
- **都传**：返回 `Validation` 错误，调用方必须明确选择一种标识方式

### D2.1: 消息级 fork 只是客户端映射，不进入服务端契约

Claude 的 transcript 分叉体验值得借鉴的是"用户从历史位置点一下就进入新分支"这一产品语义，而不是它以消息记录为真相的复制方式。AstrCode 的 durable 真相是事件流和 turn 边界，因此消息级 fork 只能是客户端便捷入口。

具体规则：
- 前端可以在带有 `turnId`、且对应 turn 已完成的历史消息上显示"从此处 fork"
- 用户点击消息级入口时，客户端先读取该消息的 `turnId`
- 客户端最终调用 `POST /api/sessions/:id/fork { turnId }`
- 服务端与 runtime 完全不知道 `messageId` 的存在，也不承担消息到 turn 的解析责任

这样可以保留 Claude 式体验，同时避免让投影层概念反向污染 runtime。

### D3: fork 逻辑放在 `SessionRuntime`，不改 `EventStore` trait

现有 `EventStore::replay()` 已经返回全量事件。fork 时 replay 后内存截断即可。

阶段一直接用 `replay()` + 内存截断。理由：
- 事件日志通常不大（几百 KB 到几 MB）
- fork 不是高频操作
- 避免改 `EventStore` trait 减少影响面

### D4: `ForkPoint` 和 `ForkResult` 放在 `session-runtime`，不污染 `core`

`core::session` 只暴露 `SessionMeta` 和 `DeleteProjectResult`——是纯粹的数据 DTO。fork 点解析是 session-runtime 的执行语义，不属于核心数据模型。`ForkPoint` 和 `ForkResult` 定义在 `session-runtime`，`application` 层通过 `session-runtime` 的公开 API 使用它们。

### D5: fork 出的新 session 是完全独立的 session

fork 后的新 session：
- 拥有独立的新 `session_id`（`generate_session_id()`）
- 独立的 `working_dir`（继承自源 session）
- 独立的事件日志文件（`event_store.ensure_session()`）
- 独立的 turn lock
- `SessionStart` 事件记录 `parent_session_id` + `parent_storage_seq`

fork 后两个 session 完全独立发展，没有任何同步或合并机制。

### D6: 复用现有谱系字段和 catalog event

`SessionStart` 已有 `parent_session_id` / `parent_storage_seq` 字段，直接复用。`SessionCatalogEvent::SessionBranched` 已有 `session_id` + `source_session_id` 字段，也直接复用。不新增任何事件类型或字段。

`SessionListItem` 的 `parentSessionId`（camelCase）字段就是谱系关系的展示出口，前端无需额外字段。

### D7: HTTP 端点设计

```
POST /api/sessions/:id/fork
Body: { turnId?: string, storageSeq?: number }
Response: SessionListItem
```

两者同时传入时返回 `400 Validation` 错误。响应通过 `SessionListItem.parentSessionId` 表达谱系关系。

前端成功收到响应后立即切换到返回的新 session，并加载其 conversation snapshot。这样 fork 在产品层表现为"从此处分叉并进入新会话"，但服务端仍保持纯粹的 session 创建语义。

### D8: 自动 branch 保持不变

现有的 `resolve_submit_target` 在 session 正忙时的自动 branch 逻辑不受影响。fork 是一个独立的、用户/后台主动触发的操作。

## Risks / Trade-offs

- [Risk] 全量 replay 后截断在超长 session 上可能有性能开销
  - Mitigation：阶段一先不优化，fork 操作频率低；后续可在 `adapter-storage` 内部加 `replay_up_to` 实现方法
- [Risk] 稳定前缀校验需要遍历事件流找到 `TurnDone` 边界
  - Mitigation：事件流已经需要 replay，遍历开销可忽略；`TurnDone` 通过 `StorageEventPayload` variant 匹配，不需要反序列化完整载荷
- [Risk] fork 后的工具持久化产物（persisted tool output）可能引用源 session 的路径
  - Mitigation：persisted output 路径是基于项目的（`FilePersistedToolOutput`），不绑定 session，不受影响
- [Risk] 前端消息级入口可能误把不稳定消息当作可 fork 点
  - Mitigation：前端只在存在 `turnId` 且该消息属于 root 已完成 turn 时显示入口；后端继续以稳定前缀规则做最终校验
