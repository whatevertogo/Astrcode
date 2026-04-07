# Runtime Session 与 Turn 生命周期设计

## 问题

这部分设计要回答两件事：

1. session / turn / subrun 的真相到底存在哪里。
2. runtime-session 这个边界到底负责什么，不负责什么。

## 核心结论

### 1. durable `StoredEvent` 是唯一事实源

- session 历史真相来自 append-only durable 事件。
- `recent_records`、replay cache、前端 render model 都只是派生层。
- compact tail 也必须基于 durable tail，而不是内存快照。

### 2. runtime-session 负责会话真相，不负责高层 read model

它负责：

- session 创建与重水合
- turn 执行边界
- durable 事件写入
- recent tail / token budget / compaction 相关状态

它不负责：

- 前端会话树
- child navigation
- 高层 subrun 列表 read model

### 3. turn 生命周期必须清晰分段

主链路保持：

- `prepare_session_execution`
- `run_session_turn`
- `execute_turn_chain`
- `complete_session_execution`

这样才能让 replay、abort、compaction、observability 都有稳定落点。

### 4. session tree 是 read model，不是领域对象

如果后面要做多会话树或 child navigation：

- 可以做 read model
- 但不能反向定义 runtime-session 的核心模型

### 5. `SharedSession` 与 `IndependentSession` 只是两种落盘语义

- `SharedSession`：子执行事件仍写入父 session
- `IndependentSession`：子执行写入独立 child session

但二者都不应该改变任务 ownership 与控制责任。

## 设计约束

- `SharedSession` 仍是正式主线
- `IndependentSession` 仍是 experimental
- session 真相、turn 生命周期、recent tail 与 replay 边界不得由前端视图反向定义

## 对应规范

- [../spec/session-and-subrun-spec.md](../spec/session-and-subrun-spec.md)
- [../spec/open-items.md](../spec/open-items.md)
