# Design: replace-summary-with-parent-delivery

## 背景

当前 child -> parent 回流依赖 `summary`、`final_reply_excerpt` 与 server 侧 summary projection。这个设计把同一件事拆成了多层弱语义：

1. child terminal result 在 `application` 层先被投影成 summary/excerpt
2. `server` 再把这些字段拼成前端可消费的 handoff/notification DTO
3. 父级 wake prompt 与前端父会话展示分别再解释一次这些字段

问题不在于“有没有总结”，而在于正式业务事实没有被显式表达。真正的事实应当是：

- child 向 direct parent 发出了一条什么消息
- 这条消息是不是 terminal
- 这条消息是 child 显式发送的，还是 runtime fallback 补投的

同时，仓库当前实现已经有两个很强的现实约束：

- `ToolDefinition` 和 capability surface 是静态的，不适合再造一套独立 `reply_to_parent` 工具来表达同一条协作主链
- `application` 现有主链已经围绕 `send_to_child`、`wake`、`terminal finalizer` 和 durable mailbox 建好，最天然的扩展点就在统一 `send`

因此，这次不再新增独立上行工具，而是让 `send` 统一承载 parent/child 两个方向的协作消息，并用 typed payload + routing invariant 维持边界。

## 目标

- 让 child 通过 unified `send` 向 direct parent 发送 `progress / completed / failed / close_request` 等 typed upward delivery。
- 让 parent wake 直接消费 typed delivery，而不是依赖 summary/excerpt 双字段。
- 删除 server 与前端合同中的 child summary 主字段，父视图直接渲染上行消息和独立子会话入口。
- 保留 deterministic fallback：如果 child 在 terminal/idle 前没有显式回父级，runtime 仍能自动投递一条保底 upward delivery。

## 非目标

- 不新增独立 `reply_to_parent` 工具。
- 不把 unified `send` 做成随意的双向聊天接口。
- 不增加“idle 后再追问 child 一轮是否完成”的额外 LLM 循环。
- 不重做通用调试 summary、session history 摘要或非 subagent 场景的 summary 模型。

## 决策

### 1. 统一用 `send`，但保持上下行合同分型

`send` 继续保留唯一工具名，但语义按“这次消息要沿协作树往哪个方向走”分流，而不是按 agent 身份二选一：

- 当参数带 `agentId` 时，`send` 表示 `-> direct child` 的具体下一步指令
- 当参数是 typed delivery payload 时，`send` 表示 `-> direct parent` 的正式上行消息

这意味着一个中间层 agent 在 Astrcode 里可以同时是：

- 自己父级的 child
- 自己子树的 parent

因此同一个 agent 在同一轮里既可能向上 `send` 汇报结果，也可能向下 `send` 继续委派，只是两次调用命中的参数分支、ownership 校验和 routing invariant 不同。

这不是“松散双向聊天”，而是“同一个协作入口名，对应两种有明确边界的业务分支”。

之所以这是最天然的实现，而不是独立 `reply_to_parent`：

- `ToolDefinition` 是静态的，单独新增一个工具会带来新的 capability、prompt surface、前端索引和测试矩阵
- 当前仓库已经围绕 `send` 建立了 reuse/ownership 心智模型；继续沿着它扩展，比多造一个并行工具更贴近现有架构
- `send_tool` 的 JSON schema 可以天然用 `oneOf` 表达 parent-downstream params 和 child-upstream params，无需改 Tool 基础设施

### 2. upward payload 仍然必须是 typed contract

虽然工具名统一成 `send`，但 child -> parent 的消息合同不能退化回普通文本。

typed parent-delivery message 至少需要明确这些结构语义：

- `kind`
- `payload`
- `terminal semantics`
- `idempotency key`
- `origin = explicit | fallback`
- `sourceTurnId`

`payload` 必须按 `kind` 做判别联合，而不是无结构 blob 或 `serde_json::Value`。`completed`、`failed`、`close_request`、`progress` 都要有最小字段集。

首批实现优先级：

- P0：`completed`、`failed`
- P1：`close_request`
- P2：`progress`

首批父视图主流程只允许依赖 P0/P1。P2 先落 durable contract、路由与序列化，不强推完整 UI timeline。

### 3. 上下行 `send` 的 routing truth 必须来自关系真相，不来自模型

unified `send` 的关键不是“工具名统一”，而是“route truth 不可伪造”。

实现必须锁死这些 invariant：

- agent 上行时，目标 direct parent 必须来自 durable parent-child 关系，而不是输入参数
- agent 下行时，目标 direct child 必须满足 direct-child ownership
- 非 root 的中间层 agent 可以同时具备上行和下行能力
- 任何方向都不允许跨树、越级或伪造 routing context
- root 只允许下行，不允许伪造上行分支
- parent 缺失、已关闭或不可达时，必须在进入 wake/finalizer 前前置拒绝
- 前置拒绝必须同时写结构化 log 和 collaboration fact

这部分 correctness 来自业务 contract + routing verification，不来自 prompt。

### 4. unreachable-parent policy 采用前置拒绝，而不是隐式兜底

结合当前 `routing.rs`、`wake.rs` 和 `recoverable_parent_deliveries` 的实现，最稳的策略不是让 child 先写半条 delivery 再让下游慢慢发现 parent 不可达，而是：

- 在 `application` 的 unified `send` child-upstream 分支先检查 direct parent handle 和 routing 条件
- 如果 parent 缺失、已终态或当前不能被视为有效 direct parent，立即拒绝
- 拒绝前写结构化 log + collaboration fact，reason code 明确

这样可以避免：

- durable event log 里出现注定无法投递的“半事实”
- wake / finalizer 层被迫猜测该不该继续处理
- handler、mapper、前端各自补脏兼容

### 5. terminal finalizer 保留 deterministic fallback

child 显式使用 unified `send` 上报才是主路径，但 runtime 仍必须保证“child 结束了，父级一定能收到结果”。

因此 finalizer 改成：

- 如果当前 child work turn 已显式写入 terminal upward delivery（`completed` / `failed` / `close_request`），finalizer 不再重复生成第二份 terminal delivery
- 如果当前 turn 进入 terminal 或 idle 但没有显式 upward delivery，finalizer 自动根据最终 assistant output 或失败事实合成 deterministic fallback delivery

这里 `sourceTurnId` 是天然必需字段。没有它，finalizer 无法可靠判断“这条 terminal reply 到底是不是当前 child work turn 发的”。

fallback 还必须满足：

- 每个 child work turn 最多一次
- 内容来源只能是 deterministic terminal fact
- 必须进入 durable event log
- 必须带 `origin = fallback`

### 6. parent wake 继续作为唯一 cross-session orchestrator

child -> parent 的 typed delivery 与 parent wake 协调仍留在 `application`：

- durable notification 先写入 parent session
- durable mailbox queue 再入队
- wake 用 delivery batch 启动 parent 后续执行

`server` 只负责 DTO 映射与传输，不再维护 child summary projection 语义。

### 7. child 可以用 `send(kind = close_request)` 申请关闭，但不能直接关闭

child 只能通过 unified `send` 发送 `close_request` 申请结束当前责任分支。

最终是否 `close` 仍由 direct parent 决策，这保持了现有 ownership 和 close 边界。

### 8. prompt 只提升命中率，不承担 correctness

prompt / governance 仍需要明确：

- 当你要把下一步具体任务交给 direct child 时，`send` 走下行分支
- 当你要把当前分支结果回给 direct parent 时，`send` 走上行分支
- 同一个中间层 agent 在同一轮里可能同时需要两种 `send`

但 prompt 只负责减少模型误用，不能承担一致性。系统一致性仍由：

- typed upward delivery contract
- routing / ownership verification
- terminal finalizer fallback
- wake / replay / idempotency 去重

### 9. live contract 可以 break，durable replay 不能失忆

本仓库不追求 live 向后兼容，但 durable session / event log 仍必须可恢复。

因此需要区分：

- live `protocol/server/frontend`：允许一次性切到新合同
- durable replay / recovery：旧 summary-backed event 必须在 replay 时被翻译或升级到新的 typed projection

策略采用 mapper upgrade，而不是长期维持新旧 live DTO 并存。

## 基于现有代码的自然实现路径

结合当前实现，最自然的改造顺序是：

1. 在 `core` 把 upward delivery model 补齐 `sourceTurnId` 等字段
2. 让 `send_tool` 接受 `oneOf` 联合 schema：
   - `agentId + message + context`
   - `kind + payload`
3. 在 `CollaborationExecutor::send` 内部保持单一入口，但 application 根据 `ToolContext.agent_context()` 分流为：
   - `send_to_child`
   - `send_to_parent`
4. 在 `terminal.rs` 通过 `sourceTurnId` 判定是否已存在显式 terminal upward delivery
5. 在 `wake.rs`、`server`、`frontend` 全部切到 typed delivery
6. 最后删除 summary 主合同

这样改动最小，也最符合现有主链形状。

## 风险与缓解

### unified `send` 的 schema 更复杂

风险：工具输入从单一 shape 变成联合 shape。

缓解：

- 保持工具名不变，但 schema 清晰分成 downstream 和 upstream 两支
- prompt guidance 必须明确“按这次协作方向选参数分支”，而不是把 agent 固定成单一角色

### child 可能误把普通文本 progress 当 completed

风险：上行 typed delivery 比 summary 更正式，误分类会直接影响 parent 决策。

缓解：

- P0/P1 优先把 terminal kinds 约束清楚
- `progress` 首批只要求 durable contract 和路由，不强推复杂 UI 语义

### replay / recovery 期间新旧事件并存

风险：旧 event log 仍然带 summary-backed event。

缓解：

- replay 阶段做 mapper upgrade
- 不删除旧事件读能力，直到所有消费者切换完毕
