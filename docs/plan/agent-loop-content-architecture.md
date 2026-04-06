# Agent Loop 内容架构

## 概要

本文档只定义 **Agent Loop 中“内容如何被表示、传输和渲染”**。

它回答的问题是：

- durable `StorageEvent` 如何投影成可消费内容
- `/messages`、`/history`、`/events` 三类接口分别承载什么内容形态
- 前端应如何把工具调用、thinking、subrun 生命周期渲染为对话内容

> 文档边界：session 真相、turn 生命周期与 replay/compaction 规则，见  
> [../design/runtime-session-and-turn-lifecycle.md](../design/runtime-session-and-turn-lifecycle.md)。

---

## 1. 范围与非目标

## 1.1 本文档覆盖范围

本文档关注三层内容视图：

1. **durable 事件层**：`StorageEvent`
2. **传输投影层**：`SessionMessage` / `SessionEventRecord`
3. **前端渲染层**：消息气泡、工具卡片、subrun 状态卡片等

## 1.2 本文档不再定义的内容

下面这些内容不再由本文档定义：

- `Session` / `SubSession` 领域模型
- `SessionRepository` 接口
- `messages.jsonl` / `sub_sessions.json` 一类文件布局真相
- session 生命周期、turn lease、recent tail、token budget
- child session tree 的后端存储结构

这些内容统一以 `runtime-session` 设计文档和真实代码为准。

---

## 2. 三层内容模型

## 2.1 durable 事件层：`StorageEvent`

运行时的源头仍然是 append-only `StorageEvent` 日志。当前与内容展示直接相关的主要事件包括：

- `UserMessage`
- `AssistantDelta`
- `ThinkingDelta`
- `AssistantFinal`
- `ToolCall`
- `ToolCallDelta`
- `ToolResult`
- `CompactApplied`
- `SubRunStarted`
- `SubRunFinished`
- `Error`
- `TurnDone`

其中：

- `SessionStart` 主要更新 session 元数据，不属于对话正文
- `PromptMetrics` 更偏执行指标，不属于常规对话内容

## 2.2 传输投影层

当前对外有两类主要投影：

### A. 稳定快照：`SessionMessage`

用于：

- `GET /api/sessions/{id}/messages`

当前快照消息主要聚合为：

- `User`
- `Assistant`
- `ToolCall`
- `Compact`

它适合作为“打开会话时的稳定初始内容”。

### B. 历史/流式事件：`SessionEventRecord` / `AgentEvent`

用于：

- `GET /api/sessions/{id}/history`
- `GET /api/sessions/{id}/events`

它保留更细粒度的执行过程，尤其适合：

- streaming 文本
- 工具流式输出
- subrun 生命周期
- 错误、完成等执行边界事件

## 2.3 前端渲染层

前端最终看到的不是原始 `StorageEvent`，而是经过归并后的 render model。这个 render model 可以来自：

- `SessionMessageDto` 的稳定快照
- `AgentEvent` 的实时增量
- 前端基于 `turn_id` / `tool_call_id` / `sub_run_id` 做的本地归并

---

## 3. 从事件到内容的映射规则

| `StorageEvent` | 主要消费者 | 典型渲染 |
|---|---|---|
| `UserMessage` | `/messages`、`/history`、`/events` | 用户消息气泡 |
| `AssistantDelta` | `/history`、`/events` | 助手消息流式正文 |
| `ThinkingDelta` | `/history`、`/events` | thinking 折叠区流式内容 |
| `AssistantFinal` | `/messages`、`/history`、`/events` | 助手最终消息 |
| `ToolCall` | `/messages`、`/history`、`/events` | 工具调用卡片 |
| `ToolCallDelta` | `/history`、`/events` | 工具输出流式增量 |
| `ToolResult` | `/messages`、`/history`、`/events` | 工具结果 |
| `CompactApplied` | `/messages`、`/history`、`/events` | compact 摘要卡片 |
| `SubRunStarted` | `/history`、`/events` | subrun 状态卡片（running） |
| `SubRunFinished` | `/history`、`/events` | subrun 状态卡片（completed/failed/aborted） |
| `Error` | `/history`、`/events` | 错误提示 |
| `TurnDone` | `/history`、`/events` | 一般不作为单独聊天内容，而是执行边界 |

### 3.1 `SubRunStarted / SubRunFinished` 的特殊地位

这两个事件很重要，因为它们承载的是：

- 子执行是否启动成功
- resolved overrides / limits
- 最终 `SubRunResult`
- `child_session_id`（若存在）
- step_count / estimated_tokens 等执行摘要

因此，前端在展示 `spawnAgent` 后续进展时，应该优先依赖：

- `SubRunStarted`
- `SubRunFinished.result`

而不是再设计一套平行的 `ChildSessionSummary` 事件。

---

## 4. 推荐的渲染模型

这里给出的是 **前端 render model**，不是要求后端立刻新增一套完全一致的 Rust 类型。

```ts
type ConversationRenderable =
  | { kind: 'message'; role: 'user' | 'assistant'; turnId?: string; blocks: RenderBlock[] }
  | { kind: 'tool'; turnId?: string; toolCallId: string; toolName: string; status: 'running' | 'done'; input?: unknown; output?: string; error?: string }
  | { kind: 'compact'; turnId?: string; summary: string; preservedRecentTurns: number }
  | { kind: 'subRun'; turnId?: string; subRunId: string; agentProfile?: string; storageMode?: 'sharedSession' | 'independentSession'; childSessionId?: string; status: 'running' | 'completed' | 'failed' | 'aborted' | 'tokenExceeded'; summary?: string; findings?: string[] }
  | { kind: 'error'; turnId?: string; message: string };

type RenderBlock =
  | { kind: 'text'; text: string }
  | { kind: 'reasoning'; text: string; collapsedByDefault: boolean };
```

### 4.1 为什么把 `subRun` 视为独立 renderable

因为它与普通 `ToolResult` 不同：

- 它有持续中的生命周期
- 它可能关联另一个 child session
- 它天然需要被点击跳转或展开
- 它的摘要中心是 `SubRunFinished.result`，不是单一字符串 output

---

## 5. `spawnAgent` 的内容语义

## 5.1 `spawnAgent` 本身仍然是普通工具调用

也就是说：

- LLM 发起 `ToolCall(tool_name = "spawnAgent")`
- 工具返回一个普通 tool result（例如 running 句柄）

## 5.2 子执行进展不靠 tool result 持续更新

后续进展应主要从生命周期事件里拿：

- `SubRunStarted`
- `SubRunFinished`

### 5.3 SharedSession 与 IndependentSession 在内容上的区别

#### SharedSession

- 子执行内容仍在父 session 的事件流里
- 需要通过 `agent.sub_run_id` / `agent.agent_id` 归并视图
- “查看子会话”本质上是 **查看同一 session 内的过滤视图**

#### IndependentSession

- 父 session 里保留 subrun 生命周期摘要
- 子执行本体在独立 child session 中继续增长
- UI 可以基于 `child_session_id` 进行跳转

---

## 6. LLM 上下文与内容投影的关系

Agent Loop 并不是直接把 UI 渲染块喂给模型；它使用的是更底层的状态投影与 prompt 组装。

但内容架构仍然需要保证下面几点：

1. **用户消息、助手最终消息、工具结果** 必须能从 durable events 中稳定重建
2. `CompactApplied` 要能被视为上下文压缩后的稳定摘要输入
3. `SubRunFinished.result` 要能成为父流程与 UI 的结构化 handoff 来源
4. `PromptMetrics`、`TurnDone`、`Error` 这类执行控制事件，除非显式需要，否则不应污染常规聊天正文

---

## 7. 加载与渲染策略

## 7.1 初始加载

建议优先使用：

- `GET /api/sessions/{id}/messages`

理由：

- 快照更稳定
- 数据量比全量历史事件更小
- 适合作为打开会话时的首屏内容

## 7.2 执行中增量

执行中建议同时订阅：

- `GET /api/sessions/{id}/events`

用于补充：

- streaming assistant/thinking
- 工具流式输出
- subrun 生命周期
- 错误与完成边界

## 7.3 需要完整执行语义时

使用：

- `GET /api/sessions/{id}/history`

它更适合：

- 复盘完整执行过程
- 首次构建 subrun 列表
- 调试 / 可观测性 / 回放场景

---

## 8. 前端渲染建议

### 8.1 thinking 默认折叠

thinking 内容应保留，但默认折叠显示，避免压过正式答案。

### 8.2 工具输出需要有“执行中”状态

因为：

- `ToolCall` 与 `ToolResult` 间存在时间差
- `ToolCallDelta` 可能持续输出

所以工具卡片应天然支持 `running → done` 的状态切换。

### 8.3 `spawnAgent` 渲染为 subrun 卡片，而不是普通长文本

推荐展示：

- agent profile
- sub_run_id
- storage mode
- 运行状态
- `SubRunFinished.result.summary`
- 如果有 `child_session_id`，显示“打开独立会话”入口
- 如果没有 `child_session_id`，显示“查看同 session 子执行视图”入口

---

## 9. 当前阶段明确不再采用的旧模型

以下内容不再作为当前内容架构主线：

- 在本文档里继续定义 `Session` / `SubSession` / `SessionRepository`
- 认为每个子 Agent 都必须创建独立 session
- 把 `messages.jsonl` 当成新的持久化真相
- 额外设计 `ChildSessionSummary` 一类平行事件来承载 subrun 摘要
- 把 session tree 直接当成 content model 的基础对象

---

## 10. 当前阶段结论

内容架构现在应当稳定在下面这条线上：

```text
StorageEvent (durable truth)
    ↓
SessionMessage / SessionEventRecord (transport projections)
    ↓
frontend renderables (message / tool / compact / subRun / error)
```

也就是说：

- session 真相属于 `runtime-session`
- subrun 生命周期属于事件层
- UI 再把这些事件归并成用户看得懂的消息与卡片
