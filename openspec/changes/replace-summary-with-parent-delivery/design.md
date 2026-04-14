# Design: replace-summary-with-parent-delivery

## 背景

当前 child -> parent 的结果回流主要依赖 `summary`、`final_reply_excerpt` 与 server 侧 summary projection。这个设计把同一件事拆成了多层弱语义：

- child terminal result 在 `application` 层先被投影成 summary/excerpt
- `server` 再把这些字段拼成前端可消费的 handoff/notification DTO
- 父级 wake prompt 与前端父会话展示分别再解释一次这些字段

这带来三个问题：

1. 协议语义重复。真正的业务事实是“child 向 direct parent 交付了什么消息”，但当前协议暴露的是“server 为 child 结果生成了什么 summary”。
2. 前端投影容易失真。父会话里展示的是 server 生成的 summary 卡片，而不是 child 的正式上行消息，导致独立子会话与父会话视图割裂。
3. 协作 contract 不完整。父级可以 `send` 给子级，但子级没有正式的上行消息动作，只能依赖 terminal finalizer 兜底写 summary。

本次变更将把 child -> parent 回流重构成显式消息合同，并删除 subagent handoff / child notification / server projection 中的 summary 主语义。

## 目标

- 让 child 通过正式业务入口向 direct parent 发送 progress / completed / failed / close_request 等 typed delivery。
- 让 parent wake 直接消费 typed delivery，而不是依赖 summary/excerpt 双字段。
- 删除 server 与前端合同中的 child summary 主字段，父视图直接渲染上行消息和独立子会话入口。
- 保留 deterministic fallback：如果 child 在 terminal/idle 前没有显式回父级，runtime 仍能自动投递一条保底 upward delivery。

## 非目标

- 不把现有 `send` 改成双向工具。
- 不增加“idle 后再追问 child 一轮是否完成”的额外 LLM 循环。
- 不把 wake / finalizer 责任下沉到 `session-runtime` 或 `kernel` 之外的其它层。
- 不重做通用调试 summary、session history 摘要或非 subagent 场景的 summary 模型。

## 决策

### 1. 新增 child-scoped upward reply，而不是复用 `send`

`send` 继续保持 `parent -> direct child` 的语义，避免双向复用后把所有权、mailbox 与协作心智模型混在一起。

新增 child-only 的正式业务入口，设计名定为 `reply_to_parent`。它只允许当前 child 向 direct parent 写入 typed delivery，不允许跨树或越级发送。

`reply_to_parent` 至少支持以下 delivery kind：

- `progress`
- `completed`
- `failed`
- `close_request`

每条 delivery 都是 durable business fact，而不是 prompt-only hint。typed parent-delivery message 至少需要明确这些结构语义：

- `kind`，防止再次退化成“所有消息都只是 message 文本”
- `payload`，承载面向 parent 的正式消息内容
- `terminal semantics`，明确该 delivery 是否结束本轮 child work turn
- `idempotency key`，用于 replay、重试与 fallback 去重
- 可选的结构化产物引用，用于 child 返回文件、报告或其它 durable artifact

`payload` 必须按 `kind` 做判别联合，而不是无结构 blob 或 `serde_json::Value` 兜底。`completed`、`failed`、`close_request`、`progress` 至少都要定义自己的最小字段集。

首批实现优先级按语义重要性分层：

- P0：`completed`、`failed`
- P1：`close_request`
- P2：`progress`

首批业务与 UI 主流程只允许依赖 P0/P1。P2 先落 durable contract、路由与序列化，不要求首批父视图完成全部 progress 语义消费。

如果首批不实现结构化产物引用，也必须在合同中预留兼容位置，而不是未来再通过自由文本补洞。

### 2. `summary` 退出 child handoff 的主合同

child completion 不再以 `summary` / `final_reply_excerpt` 作为父级回流的主字段。正式合同改为 typed parent-delivery message：

- parent wake 消费 typed delivery content
- server DTO 传输 typed delivery，而不是 child summary projection
- frontend 父会话渲染 typed delivery block，而不是 server 生成的 summary 卡片

这样 `summary` 不再承担 child handoff 的业务语义。若系统仍保留其它用途的 summary，它们也不再参与 subagent upward delivery 的协议定义。

### 3. terminal finalizer 保留 deterministic fallback

child 有显式 `reply_to_parent` 才是主路径，但 runtime 仍必须保证“child 结束了，父级一定能收到结果”。

因此统一 finalizer 改成：

- 如果当前 child work turn 已显式写入 terminal upward reply（如 `completed` / `failed` / `close_request`），finalizer 不再重复生成第二份 terminal delivery。
- 如果当前 turn 进入 terminal 或 idle 但没有显式 upward reply，finalizer 自动根据最终 assistant output 或失败事实合成一条 deterministic fallback delivery。

这里的 fallback 是 runtime 行为，不新增额外 LLM 追问回合。fallback 合同还需要明确这些触发与去重规则：

- 判定点是“当前 `turn_id` 对应的 child work turn 完成 terminal 收口时是否已经存在 terminal upward reply”，而不是单纯看 agent 是否曾进入 `Idle`
- 每个 child work turn 最多只允许一次 terminal fallback
- fallback 内容来源必须来自 deterministic terminal fact，例如最终 assistant message、失败原因、关闭原因，而不是 server 二次总结
- fallback 也必须进入 durable event log，并参与相同的 replay / 去重规则
- fallback delivery 必须带来源标记，例如 `origin = explicit | fallback`，供 parent、前端和诊断链路区分“child 主动上报”与“系统兜底补投”

### 4. parent wake 继续作为唯一的 cross-session orchestrator

child -> parent 交付与 parent wake 的协调仍留在 `application`：

- terminal / fallback 统一写入 parent delivery queue
- wake 继续通过 batch checkout / consume / requeue 驱动父级恢复
- 父级 prompt 明确感知“这是 direct child 交付给你的正式消息”

`server` 只负责 DTO 映射与传输，不再维护 child summary projection 语义。

### 5. ownership、routing 与幂等必须是显式 invariant

`reply_to_parent` 不是一个“任何 child 都能随手发消息”的宽松接口，而是 direct-parent contract。实现必须锁死这些 invariant：

- child 只能回给 direct parent
- child 不能伪造 parent routing context 或跨树回流
- reply_to_parent 的 routing truth 必须来自 durable parent-child 关系，而不是模型输入
- parent 已关闭、parent 不可达、batch 重试等异常路径必须有显式业务语义
- replay 或重试不能制造重复 terminal delivery

这部分 correctness 来自业务 contract、ownership verification 与 idempotency，不来自 prompt。

`parent` 不可达策略必须在实现前固定，禁止在 handler、mapper 或前端投影层各自临时兜底。本次 change 采用的目标方向是：保留 child 侧 durable delivery fact，并由 parent delivery queue / replay 语义决定后续恢复，而不是直接静默丢弃。

### 6. close 仍由 parent 拥有，child 只能申请

child 可以通过 `reply_to_parent(kind = close_request)` 申请关闭当前责任分支，但不能直接关闭自己或父级。

最终是否 `close` 仍由 direct parent 决策，这保持了现有 direct-parent ownership 与协作工具边界。

### 7. 前端父视图展示“消息事实”，不展示“summary 投影”

父会话需要能直接看到：

- child 显式发回的 progress/completed/failed/close_request 消息
- fallback delivery
- 独立子会话入口

父视图不再内联 server 生成的 summary 文案。需要更多细节时，用户进入子会话查看完整 transcript。

### 8. 协作 prompt 只提升命中率，不承担 correctness

child-scoped prompt contract 仍需要明确要求 child 使用 `reply_to_parent` 主动汇报结果，但它只负责提升显式上报命中率。

系统一致性仍然必须由这三层保证：

- typed upward delivery contract
- terminal finalizer fallback
- wake / replay / idempotency 去重

也就是说，即使模型没有“乖乖汇报”，系统也不能重新退回 summary 驱动的脆弱合同。

### 9. live contract 可以 break，durable replay 不能失忆

项目不追求对外向后兼容，但本地 durable session / event log 仍必须可恢复。这里需要区分两类兼容性：

- live `protocol/server/frontend` 合同：允许在同一次 change 内直接切换，不保留旧 summary 写路径
- durable replay / recovery：旧 session 中已有的 summary-backed event 仍必须可被新代码读取、投影或迁移

换句话说，本次 change 不做“新旧 live contract 长期并存”，但必须保证已有 event log 不会因为删字段而失忆。本次 change 的目标策略是 mapper upgrade：旧 summary-backed durable event 在 replay 时被翻译到新的 typed projection，而不是直接要求旧 live DTO 继续共存。

## 影响与权衡

### 优点

- 协议更干净：child -> parent 回流终于有正式消息模型。
- UI 更一致：父视图看到的是子级消息，而不是 server 二次加工摘要。
- 容错更稳：即使 child 忘记显式上报，runtime 仍能 deterministic fallback。

### 代价

- 这是一次 breaking contract 迁移，`application` / `server` / `protocol` / `frontend` 需要同一次 change 内同步切换。
- 如果 child 显式回复写得过长，父视图可能更吵，因此 prompt contract 必须要求 child 对 parent 的回信保持面向协作、简洁可执行。

## 迁移顺序

1. 引入 typed parent-delivery message 与 `reply_to_parent` 业务入口。
2. 切换 `terminal finalizer`、`wake`、`server mapper` 与 DTO，使其全部消费 typed delivery；此阶段允许保留旧 summary 读路径，但禁止新增 summary 写路径。
3. 切换前端事件与父视图渲染，不再依赖 summary。
4. 在所有消费者切换完成且 replay 策略落地后，删除 `summary` / `server summary projection` 在 child handoff 路径中的合同与测试夹具。

## 风险与缓解

### 与现有 delegation surface 变更重叠

当前仓库已经存在 `enhance-agent-tool-experience` 变更，涉及 child delegation prompt surface。本次设计通过保持 `send` 单向语义、把 upward reply 明确做成 child-scoped 新入口，降低与该变更的语义冲突。

### fallback delivery 仍可能偏冗长

fallback 只在 child 未显式上报时触发，并且来源仅限 terminal fact，不允许 server 再生成第二层 summary projection。这样即使 fallback 文案不够完美，也不会重新引入 summary 驱动的重复语义。

### close 申请与 terminal completed 可能同时出现

同一 child turn 允许先发送 `completed` 再发送 `close_request`，但最终 finalizer 只认“本轮是否已有 terminal upward reply”，避免 duplicated terminal delivery。是否真正关闭仍由 parent 在收到消息后决定。

### replay / recovery 期间新旧事件并存

旧 session 的 event log 可能仍包含 `ChildSessionNotification.summary`、`SubRunHandoff.summary` 或等价的 summary-backed 事实。实现需要明确这些旧事件是：

- 在 replay 时被翻译到新的 typed projection
- 还是通过一次性 migration 升级后再回放

但无论采用哪种路径，都不能让 durable recovery 依赖“旧字段已经不存在所以直接失败”。
