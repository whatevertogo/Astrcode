# Event Log 架构

Astrcode 采用 **Event Sourcing + CQRS Projection** 模式管理会话状态。本文档描述其设计、实现细节、以及与同类产品（Claude Code、Codex）的对比分析。

## 核心模型

### 事件存储

会话的所有状态变更以 `StorageEventPayload` 枚举变体持久化到 JSONL 文件（`adapter-storage/src/session/event_log.rs`）。

```
~/.astrcode/projects/<project-hash>/<session-id>.jsonl
```

每行是一个 `StoredEvent`：

```rust
// core/src/event/types.rs
pub struct StoredEvent {
    pub storage_seq: u64,       // 单调递增，由 writer 独占分配
    #[serde(flatten)]
    pub event: StorageEvent,    // 实际事件
}

pub struct StorageEvent {
    pub turn_id: Option<String>,
    pub agent: AgentEventContext,
    #[serde(flatten)]
    pub payload: StorageEventPayload,
}
```

`StorageEventPayload` 包含约 20 种语义事件变体：

| 类别 | 事件 |
|------|------|
| 会话生命周期 | `SessionStart`, `TurnDone` |
| 对话消息 | `UserMessage`, `AssistantFinal`, `AssistantDelta` |
| 思考过程 | `ThinkingDelta` |
| 工具交互 | `ToolCall`, `ToolCallDelta`, `ToolResult`, `ToolResultReferenceApplied` |
| 上下文管理 | `CompactApplied`, `PromptMetrics` |
| 子会话编排 | `SubRunStarted`, `SubRunFinished`, `ChildSessionNotification` |
| 协作审计 | `AgentCollaborationFact` |
| 治理模式 | `ModeChanged` |
| 输入队列 | `AgentInputQueued`, `AgentInputBatchStarted`, `AgentInputBatchAcked`, `AgentInputDiscarded` |
| 错误 | `Error` |

### 写入路径

事件写入经过 `append_and_broadcast`（`session-runtime/src/state/execution.rs`）：

```
append_and_broadcast(event)
  ├─ session.writer.append(event)           // 1. 持久化到 JSONL（fsync）
  └─ session.translate_store_and_cache()    // 2. 更新所有内存投影
       ├─ projector.apply(event)            //    a. AgentState 投影
       ├─ EventTranslator.translate()       //    b. SSE 事件转换
       ├─ recent_records/stored 缓存        //    c. 内存事件缓存
       ├─ child_nodes 更新                  //    d. 子会话树
       ├─ active_tasks 更新                 //    e. 任务快照
       └─ input_queue_projection 更新       //    f. 输入队列索引
```

关键特性：
- **单写者 + fsync**：`EventLog` 持有独占 writer，每条事件 `flush + sync_all`（`event_log.rs`）
- **Drop 安全**：`Drop` 实现中再次 flush/sync，防止进程退出时遗漏数据
- **尾部扫描**：打开文件时，大于 64KB 的文件只读取末尾 64KB 定位 `max_storage_seq`

### 状态投影

`AgentStateProjector`（`core/src/projection/agent_state.rs`）从事件流增量推导当前状态：

```rust
pub struct AgentState {
    pub session_id: String,
    pub working_dir: PathBuf,
    pub messages: Vec<LlmMessage>,   // 用于下次 LLM 请求
    pub phase: Phase,
    pub mode_id: ModeId,
    pub turn_count: usize,
    pub last_assistant_at: Option<DateTime<Utc>>,
}
```

投影器的核心行为：

1. **增量 apply**：每次事件到达时调用 `projector.apply(event)`，只更新相关字段
2. **Pending 聚合**：`AssistantFinal` 和 `ToolCall` 先进入 pending 状态，在遇到 `UserMessage`/`ToolResult`/`TurnDone`/`CompactApplied` 时 flush
3. **子会话隔离**：`should_project()` 确保 SubRun 事件不污染父会话的投影状态
4. **Compaction 回放**：`CompactApplied` 事件携带 `messages_removed`，投影时精确替换消息前缀

```rust
// 纯函数：从事件序列重建完整状态
pub fn project(events: &[StorageEvent]) -> AgentState {
    AgentStateProjector::from_events(events).snapshot()
}
```

### Compaction

上下文压缩（`session-runtime/src/context_window/compaction.rs`）通过 LLM 摘要替换投影中的消息前缀，但 **event log 原文始终保留**。

`CompactApplied` 事件记录精确的压缩边界：

```rust
CompactApplied {
    trigger: CompactTrigger,          // Auto / Manual / Deferred
    summary: String,                  // LLM 生成的摘要
    meta: CompactAppliedMeta,         // 模式、重试次数等
    preserved_recent_turns: u32,      // 保留的最近 turn 数
    messages_removed: u32,            // 精确移除的消息数（用于回放）
    pre_tokens: u32,
    post_tokens_estimate: u32,
    tokens_freed: u32,
    timestamp: DateTime<Utc>,
}
```

投影器在 apply `CompactApplied` 时：
1. 从 `messages` 头部移除 `messages_removed` 条消息
2. 插入一条 `CompactSummary` 消息作为上下文衔接
3. 保留尾部消息不变

---

## 与同类产品的对比

### 三方架构差异

| 维度 | Astrcode | Claude Code | Codex |
|------|----------|-------------|-------|
| **持久化** | JSONL event log，每条 fsync | JSONL + 100ms 批量 flush | JSONL rollout + mpsc channel 异步写 |
| **存储内容** | 语义事件（~20 种 `StorageEventPayload`） | 对话消息 + 元数据 entry | 对话消息 `ResponseItem` 数组 |
| **内存模型** | Event Sourcing + CQRS 投影 | `parentUuid` 链表回溯构建 `Message[]` | `Vec<ResponseItem>` 数组 |
| **状态来源** | `projector.apply(event)` 逐事件投影 | 从链尾回溯 `parentUuid` 链表 | 直接就是数组 |
| **Compaction** | LLM 摘要替换投影，event log 保留 | 4 级管线：snip/microcompact/collapse/autocompact | LLM 摘要 + 反向扫描 checkpoint |
| **Crash 恢复** | 最多丢 1 条事件 | 最多丢 100ms 数据 | 最多丢 channel 内 256 条 |

本质区别：Claude Code 和 Codex 存的是"说了什么"（消息），Astrcode 存的是"发生了什么"（事件）。

### Event Log 的优势

#### 1. 多投影：一份事件流驱动多个独立视图

`translate_store_and_cache` 一次写入同时维护 6 个独立投影：

- `AgentState`（消息列表，给 LLM 用）
- `child_nodes`（子 session 树，给 orchestration 用）
- `active_tasks`（任务快照）
- `input_queue_projection_index`（输入队列状态）
- `AgentCollaborationFact`（审计事实，带 mode 上下文）
- `ObservabilitySnapshot`（可观测性快照）

Claude Code 和 Codex 都是单 agent 单会话模型，状态就是消息列表。Astrcode 有 parent-child 协作树、mode 切换、policy 审计等多个正交关注点，event sourcing 让它们各自独立投影，互不耦合。

`ModeChanged` 事件是典型例子：projector 更新 `mode_id`，`current_mode` 和 `last_mode_changed_at` 字段更新，审计系统记录 mode 上下文，全部从同一个事件自然驱动。

#### 2. 横切扩展：加状态维度不需要改核心结构

添加新状态维度的路径：

1. 加一个 `StorageEventPayload` 变体
2. `AgentStateProjector.apply()` 加一个 match arm
3. 完成

旧 session 不包含新事件？自动回退到默认值，不需要数据迁移。

`ModeChanged` 就是这个路径的验证——governance mode system 从 0 到 89 个 task 的实现没有破坏任何已有状态。对比 Codex 加新状态维度需要改 `SessionState` struct、序列化格式、可能的 rollout 格式；Claude Code 需要新 entry 类型 + 改 `loadTranscriptFile` 解析逻辑 + 改链表构建逻辑。

#### 3. 审计链路完整性

`AgentCollaborationFact` 记录每次 spawn/send/close 的完整上下文——谁、在什么 mode 下、什么 policy 版本、什么 capability 面。这是事件级审计，不是消息级。

当需要回答"为什么那次 spawn 被允许/拒绝"时，event log 能直接给出答案。

#### 4. Compaction 可逆性

`CompactApplied` 记录精确的 `messages_removed`，投影器可精确回放压缩边界。原始事件从未删除。

Codex 通过 `CompactedItem.replacement_history` 存储替换后快照实现类似效果，但多了存储冗余。Claude Code 通过 `parentUuid = null` 截断链表、加载时 pre-compact skip 实现，文件内容仍在但被跳过。

三方可逆性都能达到，event sourcing 的方式最自然。

#### 5. Crash Recovery 强保证

每条事件 fsync 意味着最多丢最后一条事件。这是三者中最强的 crash recovery guarantee。

### Event Log 的劣势与优化方向

#### 劣势 1：冷启动重放成本随会话长度线性增长

`project()` 必须重放所有事件。目前没有 projection snapshot 机制——没有定期保存 `AgentState` 快照。

**优化方向**：每次 compaction 后保存 `AgentState` 快照到 `sessions/<id>/snapshots/`，冷启动从最近快照恢复，只 replay 之后的事件。

#### 劣势 2：每条 fsync 的 I/O 延迟

`append_stored` 每条事件都 `sync_all`，在高频 streaming 场景下是 I/O 瓶颈。

Claude Code 用 100ms batch drain 合并写入；Codex 用 mpsc channel（容量 256）异步写磁盘。

**优化方向**：引入 write buffer + 定时/定量 flush。关键事件（`SessionStart`、`TurnDone`）立即 flush，高频事件（`AssistantDelta`、`ToolCallDelta`）批量写入。

#### 劣势 3：Event Log 只增不减

原始消息、delta 事件、tool results 全都保留在 JSONL 中。长 session 的文件会持续增长。

**优化方向**：compaction 后截断旧事件，只保留 checkpoint marker。或定期归档冷 session 的 event log。

#### 劣势 4：投影与持久化耦合

`translate_store_and_cache` 把 persist、project、translate、cache、broadcast 五件事串行做。如果 projector 有 bug，已 fsync 的事件无法撤回，只能追加补偿事件。

#### 优势重要性评估

| 优势 | 重要程度 | 原因 |
|------|----------|------|
| 多投影 | 高 | 有 mode/policy/child/audit 四个正交关注点 |
| 横切扩展 | 高 | governance mode system 验证了这条路可行 |
| 审计链路 | 中高 | 有治理策略的系统需要可追溯决策 |
| Compaction 可逆 | 中 | 三方都能做到，event sourcing 最自然 |
| Crash recovery | 中 | 强保证好，但批量化后差异缩小 |

---

## 结论

Event sourcing 匹配 Astrcode 的系统复杂度——多 agent 协作树 + governance mode + policy engine + capability 收缩，状态维度远多于单 agent 工具。Claude Code 和 Codex 的消息数组在它们的场景下是合理选择。

需要优化的是 event log 的性能特征（写批量化、projection snapshot），不是放弃 event sourcing 本身。
