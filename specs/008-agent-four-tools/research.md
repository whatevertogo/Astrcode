# Research: Astrcode Agent 协作四工具重构

## Decision 1: 公开协作面只保留四个工具，`resume` 仅保留内部预留

**Rationale**:
- 当前 `spawnAgent`、`sendAgent`、`waitAgent`、`closeAgent`、`resumeAgent`、`deliverToParent` 六工具公开面把“消息发送”“等待”“终止”“恢复”“向上交付”拆成了多个互相耦合的概念，prompt 心智负担很高。
- 用户已经明确要求公开面收敛为 `spawn`、`send`、`observe`、`close`，并接受不做兼容入口，这让我们可以干净删除旧工具，而不是继续维护双轨 surface。
- `resume` 在 runtime 内部仍有未来价值，但它属于执行恢复策略，不属于当前公开协作最小集。

**Alternatives considered**:
- 保留 `waitAgent` 作为兼容工具：会把“消息驱动唤醒”和“父显式等待”两种模型混在一起，继续污染 prompt。
- 保留 `deliverToParent` 作为子->父专用通道：会让消息模型重新变成非对称协议，不利于统一 mailbox 设计。

## Decision 2: root agent 必须进入 `agent_control` 树，不能只停留在 `ExecutionAccepted`

**Rationale**:
- 当前 `execute_root_agent` 虽然生成了 `root_agent_id`，但 root 没有真正注册进 `agent_control`，导致 `RootExecution` 下创建 child 时无法得到稳定 `parent_agent_id_for_control`。
- 四工具要求 `send(parentId, ...)`、`observe(childId)`、`close(childId)` 都建立在统一父子树权限上；没有真实 root 节点，根层协作语义不自洽。
- 复用现有 `ancestor_chain` 与 `parent_agent_id` 比新建另一套 root 特判路由更干净，也更符合仓库“一个边界一个 owner”的原则。

**Alternatives considered**:
- 继续让 root 作为 execution 级概念存在，子树内再单独维护 control 关系：会形成两套父子真相源。
- 为 root 做单独的旁路权限判断：短期少改，但会让 send/observe/close 在根层与非根层出现不同语义。

## Decision 3: 将 `AgentStatus` 拆成生命周期状态与最近一轮结果

**Rationale**:
- 旧 `AgentStatus` 同时承担“这个 agent 还活着吗”和“上一轮执行结果是什么”两种语义，无法表达“单轮完成但 agent 仍然可复用”的四工具模型。
- 子 agent 单轮完成后必须回到 `Idle`，否则 `send(childId, ...)` 就会变成“发给一个已结束的对象”。
- 把生命周期与最近一轮结果分开后，`observe` 才能同时回答“它现在是否可接收新消息”和“上一轮是成功还是失败”。

**Alternatives considered**:
- 保留一个枚举，额外发明更多复合状态：会把状态空间炸得更复杂，而且仍然难以在 `observe` 中清晰表达。
- 用字段组合但不改 `SubRunHandle`：会让 live handle 与 DTO 继续语义不一致。

## Decision 4: mailbox durable 化直接走 session event log，而不是新建 WAL 或 message router

**Rationale**:
- Astrcode 已经有 session event log 作为 durable 真相源；如果再新建独立 WAL，会引入第二套恢复语义和第二个 durable owner。
- 当前存储层只有单事件 `append`，没有事务批写；这意味着我们必须接受 Started/Acked 之间存在 crash 窗口，而不是假装可以做 exactly-once。
- 通过新增 `AgentMailboxQueued`、`AgentMailboxBatchStarted`、`AgentMailboxBatchAcked`、`AgentMailboxDiscarded`，可以在不改变底层存储模型的前提下得到可重放 mailbox。

**Alternatives considered**:
- 新建独立 router + WAL：对全新项目合理，但会让当前仓库出现两套 durable 事件系统。
- 继续只用 live inbox：运行时重启后 pending message 会直接丢失，不满足本特性的可靠性目标。

## Decision 5: mailbox 语义明确采用 `at-least-once`，并以稳定 `delivery_id` 识别重复

**Rationale**:
- 在当前单事件 `append` 约束下，不可能免费得到 exactly-once；如果在注入前就 ack，会引入 at-most-once 的静默丢失风险。
- 选择 `at-least-once` 后，可以把 `BatchStarted` 定义为“本轮接管了哪些消息”，把 `BatchAcked` 定义为“durable turn completion 后确认完成处理”，逻辑更自洽。
- 稳定 `delivery_id` 能让服务端与模型同时识别“这是一条恢复后的重复消息”，而不是依赖文本相似度猜测。

**Alternatives considered**:
- 在 `BatchStarted` 时就视为最终确认：崩溃后会永久丢失这批消息。
- 尝试依赖 context window 中的历史注入做去重：动态 prompt 注入不是 durable transcript，重启后上下文不可靠。

## Decision 6: turn 只消费 `turn-start snapshot drain` 的固定 batch

**Rationale**:
- 如果 turn 运行中到达的新消息也能混入当前轮，上下文边界就不稳定，`BatchStarted` 的语义也无法定义清楚。
- snapshot drain 能让 `activeTask`、`pendingTask`、`pendingMessageCount` 都有稳定来源，也更利于调试和复现。
- 这条规则同时适用于父发子和子发父，是整个 mailbox 调度稳定性的核心约束。

**Alternatives considered**:
- 边跑边持续吸收 mailbox：会让模型上下文在同一轮里不断变化，调试和持久化都非常困难。
- 每条消息单独触发一轮：会放大 wake 开销，也让批量协作失去意义。

## Decision 7: mailbox 投影独立建模，不污染现有 `AgentStateProjector`

**Rationale**:
- 当前 `AgentStateProjector` 已经承担 phase、turn_count、输出摘要等对话投影职责，再硬塞 pending/active batch 和 delivery replay，会让 boundary owner 混乱。
- `observe` 需要同时拼接 live handle、对话投影和 mailbox 摘要，这更适合通过轻量 `MailboxProjector` 或等价 replay 逻辑完成。
- 这样可以让 mailbox 演进与聊天内容投影解耦，不破坏现有 `/history`、`/events` 心智。

**Alternatives considered**:
- 直接往 `AgentStateProjector` 里加 mailbox 字段：短期少建模块，但会把 durable mailbox 和聊天语义缠在一起。
- `observe` 全靠 live state 计算：重启后无法稳定恢复 pending message count 和 active/pending task。

## Decision 8: 新 child 一律写为 `IndependentSession`，`SharedSession` 只保留历史读取

**Rationale**:
- `runtime-execution/src/policy.rs` 已经把 `IndependentSession` 作为当前默认方向，说明仓库已经在向 ownership/storage 解耦靠拢。
- mailbox durable 事件和 root/child 控制树更适合围绕独立会话组织；继续生成新的 `SharedSession` 只会增加迁移分支。
- 用户已经明确希望删除 `SharedSession` 新写路径，只保留历史可读性。

**Alternatives considered**:
- 继续允许新 child 按策略写成 `SharedSession`：会让 mailbox durable 归属和 lineage 表达变复杂。
- 一次性清除所有 legacy `SharedSession` 读取逻辑：风险过高，也超出本特性的必要范围。
