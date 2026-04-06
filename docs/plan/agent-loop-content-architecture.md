# Agent Loop 内容架构

## 概要

本文档只定义 **Agent Loop 中“内容如何被表示、传输和归并”**。

它回答的问题是：

- durable `StorageEvent` 如何成为唯一事实源
- `AgentEventEnvelope` 如何作为统一传输协议
- 前端应如何把工具调用、thinking、compact、subrun 生命周期归并成 render model

> 文档边界：session 真相、turn 生命周期与 replay/compaction 规则，见  
> [../design/runtime-session-and-turn-lifecycle.md](../design/runtime-session-and-turn-lifecycle.md)。

---

## 1. 范围与非目标

## 1.1 本文档覆盖范围

本文档关注三层内容模型：

1. **durable 事件层**：`StorageEvent`
2. **传输事件层**：`AgentEvent` / `AgentEventEnvelope`
3. **前端归并层**：message / tool / subRun / compact / error 等 render model

## 1.2 本文档不再定义的内容

下面这些内容不再由本文档定义：

- `Session` / `SubSession` 领域模型
- `SessionRepository` 接口
- `messages.jsonl` / `sub_sessions.json` 一类文件布局真相
- session 生命周期、turn lease、recent tail、token budget
- child session tree 的后端存储结构

这些内容统一以 `runtime-session` 设计文档和真实代码为准。

---

## 2. 规范性说明（非常重要）

为避免“后端契约”和“前端参考实现”混杂，本文档采用下面的约束：

### 2.1 后端契约（Normative）

以下内容属于后端必须稳定遵守的契约：

- `StorageEvent` / `AgentEvent` / `AgentEventEnvelope` 的事件语义
- `turn_id` / `agent_id` / `sub_run_id` / `child_session_id` 的含义
- `SubRunStarted / SubRunFinished` 的生命周期语义
- `CompactApplied` 的语义
- `/history` 与 `/events` 的统一事件协议

### 2.2 前端参考（Informative）

以下内容属于前端参考实现，而不是后端强制契约：

- render model 的具体 TypeScript 结构
- 消息气泡 / 工具卡片 / compact 卡片 / subrun 卡片的展示方式
- thinking 默认折叠、breadcrumb、过滤视图等 UI 决策

---

## 3. 三层内容模型

## 3.1 durable 事件层：`StorageEvent`

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

## 3.2 传输事件层：`AgentEventEnvelope`

当前主线传输协议应统一为事件信封：

- `GET /api/sessions/{id}/history`
- `GET /api/sessions/{id}/events`

二者都返回同一套 `AgentEventEnvelope` 语义：

- `/history`：首屏 hydration / 回放
- `/events`：SSE 增量

### 3.2.1 `/messages` 的当前定位

`GET /api/sessions/{id}/messages` 已经删除，**不再作为兼容接口存在**。

原因：

- 它是后端特制的 UI 快照投影
- 随事件类型增长，归并逻辑会持续膨胀
- 它已经无法完整覆盖 subrun / compact 等真实语义
- 前端若同时消费 `/messages` 与 `/history`，将被迫维护两套归并逻辑

因此当前设计结论是：

- **主线：只使用 `/history + /events`**
- `/messages` 不再维护
- 首屏 hydration 与增量同步统一到 `/history + /events`

## 3.3 前端归并层

前端最终看到的不是原始 `StorageEvent`，而是基于统一事件协议归并出的 render model。

这层 render model：

- 可以自由演进
- 可以按视图目标（根会话 / subrun 过滤视图 / child session）做不同组合
- 但不能反向定义后端事实层

---

## 4. 从事件到内容的映射规则

| `StorageEvent` | 主要消费者 | 典型渲染 |
|---|---|---|
| `UserMessage` | `/history`、`/events` | 用户消息气泡 |
| `AssistantDelta` | `/history`、`/events` | 助手消息流式正文 |
| `ThinkingDelta` | `/history`、`/events` | thinking 折叠区流式内容 |
| `AssistantFinal` | `/history`、`/events` | 助手最终消息 |
| `ToolCall` | `/history`、`/events` | 工具调用卡片 |
| `ToolCallDelta` | `/history`、`/events` | 工具输出流式增量 |
| `ToolResult` | `/history`、`/events` | 工具结果 |
| `CompactApplied` | `/history`、`/events` | compact 边界 / 摘要提示 |
| `SubRunStarted` | `/history`、`/events` | subrun 状态卡片（running） |
| `SubRunFinished` | `/history`、`/events` | subrun 状态卡片（completed/failed/aborted） |
| `Error` | `/history`、`/events` | 错误提示 |
| `TurnDone` | `/history`、`/events` | 一般不作为单独聊天内容，而是执行边界 |

### 4.1 `CompactApplied` 的语义

`CompactApplied` 的核心语义不是“消息内容”，而是：

- 上下文已经发生压缩
- 之前的历史语义上被摘要替代
- 后续 prompt 组装会基于 compact 后的边界继续前进

因此：

- 在**事件层**，它只是普通事件
- 在**前端层**，它可以被渲染成摘要卡片 / 分隔线 / 折叠提示
- 但它不应被后端快照协议强行定义成和 user/assistant 平级的“对话消息”

### 4.2 `SubRunStarted / SubRunFinished` 的特殊地位

这两个事件承载的是子执行生命周期真相：

- 子执行是否启动成功
- resolved overrides / limits
- 最终 `SubRunResult`
- `child_session_id`（若存在）
- `step_count` / `estimated_tokens` 等执行摘要

因此父流程与前端在消费 `spawnAgent` 结果时，应该优先依赖：

- `SubRunStarted`
- `SubRunFinished`
- `SubRunFinished.result`

而不是再设计一套平行的 `ChildSessionSummary` 事件。

---

## 5. `spawnAgent` 的内容语义

## 5.1 `spawnAgent` 本身仍然是普通工具调用

也就是说：

- LLM 发起 `ToolCall(tool_name = "spawnAgent")`
- 工具返回一个普通 tool result（通常是 running 句柄与 artifact 引用）

## 5.2 子执行进展不靠 tool result 持续更新

后续进展应主要从生命周期事件里拿：

- `SubRunStarted`
- `SubRunFinished`

### 5.2.1 前端不应硬编码 `tool_name == "spawnAgent"` 才识别 subrun

前端识别“这是一个子执行”的核心依据应是生命周期事件本身，而不是工具名字。

也就是说：

- tool card 可以先作为普通工具调用出现
- 收到 `SubRunStarted` 后再升级/关联为 subrun 卡片

### 5.2.2 但协议必须定义稳定关联规则

仅靠“同一 turn 内出现了 `spawnAgent` 与 `SubRunStarted`”还不够稳，因为：

- 同一 `turn_id` 内可能出现多个 `spawnAgent`
- 若没有稳定关联键，前端无法确定哪个 tool card 对应哪个 subrun

因此文档上必须明确下面两种方案中的至少一种：

1. **首选方案**：在 subrun 生命周期事件中显式携带 `tool_call_id`
2. **兼容方案**：明确规定“同一 turn 内按发出顺序 1:1 配对”

当前建议是：**优先补 `tool_call_id` 级别的稳定关联**，避免前端靠顺序猜测。

---

## 6. SharedSession 与 IndependentSession 的内容差异

## 6.1 SharedSession

- 子执行内容仍在父 session 的事件流里
- 前端看到的是 **同一 session 内的子执行树 / 过滤视图**
- 如果要做 server-side 过滤，过滤语义不应只是 `sub_run_id == 当前值`

### 6.1.1 为什么简单 `subRunId` equality filter 不够

如果当前视图正在看 `subrun-a`，它下面还有 `subrun-b`：

- 只过滤 `agent.sub_run_id == subrun-a`
- 会把 `subrun-b` 的生命周期和内容全部裁掉

这会破坏嵌套子执行树。

因此更合理的过滤语义应是：

- `scope=self`
- `scope=subtree`
- `scope=directChildren`

也就是：

- `subRunId=subrun-a&scope=self`
- `subRunId=subrun-a&scope=subtree`
- `subRunId=subrun-a&scope=directChildren`

其中主线更推荐 `subRunId + scope`，而不是把 `agent_id` 暴露为主要过滤键。

## 6.2 IndependentSession

- 父 session 中仍保留 subrun 生命周期摘要
- 子执行本体在独立 child session 中继续增长
- `child_session_id` 是跳转入口
- 父侧对子执行的结构化结果仍优先来自 `SubRunFinished.result`

---

## 7. 推荐的前端 render model（非契约）

这里给出的是 **前端参考结构**，不是要求后端新增一套完全一致的 Rust 类型。

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

### 7.1 为什么把 `subRun` 视为独立 renderable

因为它与普通 `ToolResult` 不同：

- 它有持续中的生命周期
- 它可能关联另一个 child session
- 它天然需要被点击跳转或展开
- 它的摘要中心是 `SubRunFinished.result`，不是单一字符串 output

---

## 8. LLM 上下文与内容投影的关系

Agent Loop 并不是直接把 UI 渲染块喂给模型；它使用的是更底层的状态投影与 prompt 组装。

但内容架构仍然需要保证下面几点：

1. 用户消息、助手最终消息、工具结果必须能从 durable events 中稳定重建
2. `CompactApplied` 要能作为“上下文边界变化”被稳定表达
3. `SubRunFinished.result` 要能成为父流程与 UI 的结构化 handoff 来源
4. `PromptMetrics`、`TurnDone`、`Error` 这类执行控制事件，除非显式需要，否则不应污染常规聊天正文

---

## 9. 加载与渲染策略

## 9.1 初始加载

主线建议使用：

- `GET /api/sessions/{id}/history`

理由：

- 与 `/events` 使用同一协议
- 避免维护第二套 `/messages` 快照归并逻辑
- 能完整覆盖 subrun / compact / streaming 相关语义

## 9.2 执行中增量

执行中订阅：

- `GET /api/sessions/{id}/events`

用于补充：

- streaming assistant/thinking
- 工具流式输出
- subrun 生命周期
- 错误与完成边界

## 9.3 需要性能优化时

当前服务端已经支持按 subrun 过滤的优化查询：

- `GET /api/sessions/{id}/history?subRunId=...&scope=...`
- `GET /api/sessions/{id}/events?subRunId=...&scope=...`

它属于优化项，不改变主线协议。

---

## 10. 前端渲染建议（非契约）

### 10.1 thinking 默认折叠

thinking 内容应保留，但默认折叠显示，避免压过正式答案。

### 10.2 工具输出需要有“执行中”状态

因为：

- `ToolCall` 与 `ToolResult` 间存在时间差
- `ToolCallDelta` 可能持续输出

所以工具卡片应天然支持 `running → done` 的状态切换。

### 10.3 `spawnAgent` 渲染为 subrun 卡片，而不是普通长文本

推荐展示：

- agent profile
- sub_run_id
- storage mode
- 运行状态
- `SubRunFinished.result.summary`
- 如果有 `child_session_id`，显示“打开独立会话”入口
- 如果没有 `child_session_id`，显示“查看同 session 子执行视图”入口

---

## 11. 当前阶段明确不再采用的旧模型

以下内容不再作为当前内容架构主线：

- 把 `/messages` 当成主线首屏协议继续扩展
- 在本文档里继续定义 `Session` / `SubSession` / `SessionRepository`
- 认为每个子 Agent 都必须创建独立 session
- 把 `messages.jsonl` 当成新的持久化真相
- 额外设计 `ChildSessionSummary` 一类平行事件来承载 subrun 摘要
- 把 session tree 直接当成 content model 的基础对象

---

## 12. 当前阶段结论

内容架构现在应当稳定在下面这条线上：

```text
StorageEvent (durable truth)
    ↓ EventTranslator
AgentEvent
    ↓ mapper
AgentEventEnvelope (/history + /events 的统一协议)
    ↓
frontend render model (message / tool / compact / subRun / error)
```

也就是说：

- `StorageEvent` 是唯一事实源
- `/history` 与 `/events` 才是主线协议
- `/messages` 应进入废弃路径，而不是继续修补
- 后端负责把事实完整投影出来
- 前端负责归并与渲染
