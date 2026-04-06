# Runtime Session 与 Turn 生命周期设计

## 1. 文档目的

本文档定义 **Astrcode 当前运行时中 session 语义的唯一设计锚点**，用于回答下面几类问题：

- session 在运行时里的“真相”由谁持有
- turn 从开始到结束经过哪些阶段
- event append / broadcast / replay / recent tail 的职责边界在哪里
- subrun、child session、storage mode 与 session 的关系是什么

> 文档边界：本文描述的是 **runtime/backend 侧 session 真相**。  
> `spawnAgent` 工具面与 API 面，见 [agent-tool-and-api-design](./agent-tool-and-api-design.md)。  
> 内容块与消息投影，见 [../plan/agent-loop-content-architecture.md](../plan/agent-loop-content-architecture.md)。  
> 前端导航与多会话视图，见 [multi-session-frontend-architecture](./multi-session-frontend-architecture.md)。

---

## 2. 当前代码锚点

| 位置 | 当前职责 |
|---|---|
| `crates/runtime-session/src/lib.rs` | 对外导出 session 真相对象与 turn 生命周期辅助函数 |
| `crates/runtime-session/src/session_state.rs` | `SessionState`、`SessionWriter`、`SessionStateEventSink`、recent caches |
| `crates/runtime-session/src/turn_runtime.rs` | `prepare_session_execution`、`run_session_turn`、`execute_turn_chain`、`complete_session_execution` |
| `crates/runtime/src/service/session_service.rs` | session 创建、重水合、快照/历史读取；负责把 durable event log 重建为 `SessionState` |
| `crates/storage/...` | event log 持久化实现；不持有 in-memory session truth |

这意味着：

- **代码层面 session 已经拆到 `astrcode-runtime-session` crate**
- 现在缺的不是再拆一个 crate，而是补一份清晰的 **source of truth 文档**

---

## 3. 核心术语

| 术语 | 定义 | 备注 |
|---|---|---|
| Session | 一个 append-only event log + 对应的 in-memory `SessionState` | 是运行时与 SSE 的基本归属单位 |
| Turn | 一次 session 内的单轮执行链 | 通常从用户消息开始，以 `TurnDone` 结束 |
| SubRun | 一次由 `spawnAgent` 触发的受控子执行 | 由 `sub_run_id` 标识，不等于 child session |
| Child Session | 只有在 `IndependentSession` 模式下才会创建的独立 session | 通过 `child_session_id` 关联 |
| SharedSession | 子执行事件仍写入父 session | 当前正式主线 |
| IndependentSession | 子执行拥有独立 child session | 当前仍属 experimental |

### 3.1 一个容易混淆但必须固定的区别

- **不是每个 subrun 都会生成 child session**
- **不是每个 child session 字段都应该回写进 `SessionMeta`**
- `SessionMeta.parent_session_id` 当前是 **session 级谱系字段**，主要服务于 session 分叉/来源；它**不是**通用的 subrun tree read model

---

## 4. `astrcode-runtime-session` 的职责边界

## 4.1 它负责什么

`astrcode-runtime-session` 负责：

1. **持有 session in-memory truth**
   - 当前 phase
   - 是否正在运行 turn
   - 当前 cancel token / active turn id / turn lease
   - token budget 与 auto-continue 计数
2. **把 durable event 追加到当前 session，并同步广播到订阅者**
3. **维护投影与 recent caches**
   - `AgentStateProjector`
   - recent `SessionEventRecord`
   - recent `StoredEvent`
4. **提供 turn 生命周期辅助**
   - 进入运行态
   - 执行 turn chain
   - 注入 auto-continue nudge
   - 收尾与清理
5. **为 compaction / replay 提供 durable-tail 辅助函数**

## 4.2 它不负责什么

它**不负责**：

- `RuntimeService` 的 façade 组装
- HTTP/SSE 路由与 DTO 映射
- 具体 event log 持久化实现
- agent profile 解析与工具注册
- root/subrun orchestration 策略本身
- 前端 session tree / breadcrumb / UI 视图模型

### 4.3 与相邻 crate 的边界

| Crate | 边界 |
|---|---|
| `astrcode-core` | 定义 `StorageEvent`、`AgentEvent`、`Phase`、`SessionTurnLease` 等契约 |
| `astrcode-storage` | 实现 event log 读写与 session 元数据查询 |
| `astrcode-runtime-agent-loop` | 负责单轮 agent loop 如何跑 |
| `astrcode-runtime-agent-control` | 负责多 agent / subrun 的控制面、取消传播与状态追踪 |
| `astrcode-runtime` | 作为 façade 组合 session、storage、execution、server-facing services |

---

## 5. 核心对象

## 5.1 `SessionWriter`

`SessionWriter` 是对 `EventLogWriter` 的线程安全包装，负责：

- 串行追加 `StorageEvent`
- 生成单调递增的 `StoredEvent`
- 在 async 场景中通过 `spawn_blocking` 包装阻塞写入

它的设计目标很单一：**session 内只有一条 durable append path**。

## 5.2 `SessionState`

`SessionState` 是 session 的 in-memory 真相。当前关键字段如下：

| 字段 | 作用 |
|---|---|
| `phase` | 当前 session phase |
| `running` | 当前是否存在活动 turn |
| `cancel` | 当前 turn 的取消令牌 |
| `active_turn_id` | 当前 turn id |
| `turn_lease` | 外部持有的 turn lease，防止并发 turn |
| `token_budget` | token 预算累计状态 |
| `compact_failure_count` | compact 失败统计 |
| `broadcaster` | SSE / 订阅者使用的广播通道 |
| `writer` | durable append 入口 |
| `projector` | `AgentStateProjector`，从事件重建 agent state |
| `recent_records` | 最近的 `SessionEventRecord` 缓存 |
| `recent_stored` | 最近的 `StoredEvent` 缓存 |

### 5.2.1 为什么同时保留 `recent_records` 和 `recent_stored`

两者服务不同：

- `recent_records`：给 SSE 断点续传 / 最近事件追赶使用
- `recent_stored`：给 compaction tail / durable replay 语义使用

这两个缓存不能互相替代，因为：

- `SessionEventRecord` 已经是投影后的事件记录
- compaction 需要基于 durable `StoredEvent` 重新截取真实 tail

## 5.3 `SessionStateEventSink`

`SessionStateEventSink` 实现了 `ToolEventSink`，它把工具执行过程中产生的 `StorageEvent` 直接接到 session append+broadcast 链路上。

它的职责不是解释业务语义，而是保证：

- 工具回调里发出的 event 也遵守同一条 durable append path
- translator / projector / broadcaster 的状态与正常 turn 输出保持一致

## 5.4 `SessionTokenBudgetState`

`SessionTokenBudgetState` 当前只承载 turn 级连续执行辅助所需的最小状态：

- `total_budget`
- `used_tokens`
- `continuation_count`

它属于 **当前活动 turn 的临时执行态**，不会成为独立 session 元数据。

---

## 6. Turn 生命周期

## 6.1 session 创建与重水合

`runtime-session` 不直接负责“发现 session 文件并加载它”，这一步在 `SessionService` 中完成：

1. 从 storage 读出 durable `StoredEvent` 序列
2. 以事件序列重建 `AgentStateProjector`
3. 构造 `SessionWriter`
4. 用最近 `StoredEvent` / `SessionEventRecord` 初始化 `SessionState`

也就是说：

- **session 真相运行在 `SessionState` 中**
- **session 真相的 durable 来源仍然是 append-only event log**

## 6.2 执行路径总览

```text
SessionService / Runtime façade
    ↓
prepare_session_execution
    ↓
run_session_turn
    ↓
execute_turn_chain
    ├─ append_and_broadcast(... user/tool/assistant/subrun events ...)
    ├─ maybe_continue_after_turn
    └─ append TurnDone
    ↓
complete_session_execution
```

## 6.3 `prepare_session_execution`

进入 turn 前，运行时会：

- 抢占并记录当前 `CancelToken`
- 设置 `active_turn_id`
- 安装 `turn_lease`
- 把 `running` 置为 true
- 初始化 token budget（如果本轮配置了 budget）

这个阶段的核心不变量是：

- **同一 session 同时只允许一个活动 turn**
- 如果 `running` 已经是 true，再次进入要报错，而不是静默覆盖状态

## 6.4 `run_session_turn`

`run_session_turn` 做两件事：

1. 先把本轮入口事件（通常是用户消息）写入 session
2. 再调用 `execute_turn_chain` 执行真正的 agent loop

如果执行失败，它会补发：

- `StorageEvent::Error`
- `StorageEvent::TurnDone { reason: "error" }`

这样做的目标是：**即使失败，event log 里的 turn 边界仍然闭合**。

## 6.5 `execute_turn_chain`

`execute_turn_chain` 是 turn 生命周期的核心执行循环：

1. 从 `projector` 快照出当前 `AgentState`
2. 从 recent durable events 中截取 compaction tail seed
3. 调用 `AgentLoop::run_turn_without_finish_with_compaction_tail(...)`
4. 在回调里对每个 `StorageEvent` 执行统一的 `append_and_broadcast`
5. 记录 prompt metrics / assistant output，用于 token budget 决策
6. 如果满足 auto-continue 条件，则注入 `AutoContinueNudge` 用户消息并继续下一轮
7. 最终补发 `TurnDone`

### 6.5.1 为什么回调里直接 append event

因为对子循环中的每一个 event，运行时都必须同时保证：

- durable log 已落盘
- `AgentStateProjector` 已应用
- recent cache 已更新
- SSE 订阅者已看到同一序列

这是 `append_and_broadcast` 存在的原因。

## 6.6 `complete_session_execution`

turn 收尾时，运行时会：

- 更新 session phase
- 清空 `active_turn_id`
- 调用 `agent_control.cancel_for_parent_turn(turn_id)`，清理父 turn 关联的子执行
- 释放 `turn_lease`
- 清空 token budget
- 重置 cancel token
- 把 `running` 置回 false

这一步要保证：**所有 turn-scoped 状态都被显式清理，而不是依赖下次覆盖**。

---

## 7. 事件缓存、回放与 compaction tail

## 7.1 durable source 永远是 `StoredEvent`

即使内存里有 `projector`、`recent_records`、`recent_stored`，durable 真相仍然只有一份：

- append-only JSONL event log

因此：

- 内存缓存只能加速 replay / SSE catch-up
- 一旦缓存不足，必须允许回退到磁盘重放

## 7.2 recent records 的语义

`SessionState::recent_records_after(last_event_id)` 会在以下场景返回 `None`：

- 缓存已经截断
- 调用方请求的事件位置早于当前缓存最早事件

这个 `None` 的含义不是“没有新事件”，而是：

- **内存缓存已经不足，调用方必须回退到 durable replay**

## 7.3 compaction tail 只基于 durable tail

`recent_turn_event_tail()` 当前只保留这些 event：

- `UserMessage`
- `AssistantFinal`
- `ToolCall`
- `ToolResult`

并按“最近 N 个用户 turn”截断。

关键原则：

- **compact 的 tail 选择必须基于 durable events，而不是 UI 消息列表**
- 否则 shared session / subrun /工具流式输出等场景下会丢失真实执行边界

---

## 8. Session、SubRun 与 Child Session 的关系

## 8.1 `SharedSession`

在 `SharedSession` 模式下：

- 子执行事件仍写入父 session
- 事件通过 `AgentEventContext.sub_run_id` 区分归属
- `child_session_id` 通常为空
- 前端看到的应该是 **同一 session 内的 subrun 视图**，不是新的 session

## 8.2 `IndependentSession`

在 `IndependentSession` 模式下：

- 父 session 中仍会有 `SubRunStarted / SubRunFinished`
- 子执行本身拥有独立 child session event log
- `child_session_id` 会成为跳转入口
- 但父侧对子执行的“结构化真相”仍然优先来自 `SubRunFinished.result`

## 8.3 一个必须遵守的建模规则

如果后续要做多会话树或 child navigation：

- **session tree 是上层 read model，不是 runtime-session 的核心模型**
- runtime-session 只保证 session 真相与 subrun 事件边界
- tree、breadcrumb、children 列表都应该建立在事件/投影之上，而不是反向污染 `SessionState` 或 `SessionMeta`

---

## 9. 对外文档与 API 的约束

## 9.1 对文档的约束

- `agent-loop-content-architecture.md` 只讨论内容投影，不再重新定义 `Session/SubSession/SessionRepository`
- `multi-session-frontend-architecture.md` 只讨论前端导航与视图模型，不再把 `SessionMeta` 直接扩展成 session tree source of truth
- `agent-tool-and-api-design.md` 继续定义 `spawnAgent`、`SubRunStarted`、`SubRunFinished` 与 API surface，但不重复定义 turn 生命周期细节

## 9.2 对 API 设计的约束

多会话能力下一步更适合先补：

- `GET /api/v1/sessions/{id}/subruns`

而不是一开始就做：

- `GET /api/v1/sessions/tree`

原因是：

1. `subrun` 是当前主线中真实稳定的执行对象
2. `tree` 只是更高层的导航投影
3. `subrun` read model 可以先基于 durable `SubRunStarted / SubRunFinished` 事件重建，再按需用 live `AgentControl` 丰富运行中状态

---

## 10. 当前阶段结论

### 10.1 现在应该固定的结论

- session 代码边界已经拆到 `astrcode-runtime-session`
- 当前最需要补的是 **设计锚点文档**，不是继续拆 crate
- `SharedSession` 仍是正式主线
- `IndependentSession` 仍应视为 experimental 扩展面
- `session tree` 应当是 read model，而不是基础领域对象

### 10.2 后续开放问题

1. 是否需要 durable 的 `list_subruns(session_id)` read model
2. 是否需要 server-side `sub_run_id` 过滤来优化 SSE/历史查询
3. `IndependentSession` 何时从 experimental 升级为正式路径
4. 是否需要把 session compaction / replay cache 指标进一步下沉到独立 service 文档
