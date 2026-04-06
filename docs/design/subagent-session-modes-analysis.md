# Astrcode 多 Agent 会话模式：对 Claude 设计的采纳与边界

> 最后更新：2026-04-05
> 结论：Astrcode 吸收 Claude 的“多维 override 分层”思想，但不复制其自由共享父运行时状态的模型。

---

## 1. 当前实现状态

Astrcode 当前已经不是早期的 `isolated_session` 二元模型，而是受控子会话（controlled sub-session）骨架：

- 已有 `sub_run_id`，显式区分“agent 实例”和“执行域实例”
- 已有 `InvocationKind`，区分 `SubRun` 与 `RootExecution`
- 已有 `SubRunStarted / SubRunFinished` 生命周期事件
- 已有 `shared_session / independent_session` 两种存储落点
- 已有 `POST /api/v1/agents/{id}/execute`
- 已有 `GET /api/v1/sessions/{id}/subruns/{sub_run_id}`

当前姿态保持不变：

- `shared_session` 是正式路径
- `independent_session` 是 experimental
- 子 agent 仍是“受控子会话”，不是普通 tool call，也不是默认独立 session

---

## 2. Claude 设计带来的启发

Claude Code 最有价值的点，不是“共享得更自由”，而是把共享拆成多个维度，而不是只靠一个模式开关：

- 存储与转录是否分离
- 上下文是否继承
- 取消与任务控制是否联动
- 指标与结果是否聚合
- 缓存与优化状态是否复用

这说明 Astrcode 后续也不能把所有语义继续堆进 `storage_mode`。`storage_mode` 只该回答一个问题：**事件写到哪个 session，以及子执行域归属哪个 session**。

---

## 3. Astrcode 的采纳 / 暂缓 / 拒绝矩阵

| 类别 | 内容 | 结论 | 原因 |
|---|---|---|---|
| 采纳 | 多维 override 分层思路 | ✅ 采纳 | 这能避免 `storage_mode` 继续膨胀为“大一统语义开关” |
| 采纳 | 任务控制通道永不隔离 | ✅ 采纳 | kill / cleanup / timeout 不应依赖 session sharing 语义 |
| 采纳 | shared observability | ✅ 采纳 | token/step/outcome/findings 适合聚合，但不需要共享可变状态 |
| 暂缓 | file read cache sharing | 🟡 暂缓 | 更像执行优化，不该先进入公开语义面 |
| 暂缓 | content replacement state sharing | 🟡 暂缓 | 与 replay / debug / compact 强相关，边界不清前先不开放 |
| 暂缓 | prompt-cache 命中优化 | 🟡 暂缓 | 只有不改变行为语义时才适合下沉到 runtime optimizer |
| 拒绝 | `shareSetAppState` 风格的父状态直写 | ❌ 拒绝 | 会破坏 Astrcode 当前事件清晰、可回放的架构优势 |
| 拒绝 | `getAppState` / 权限提示直通 | ❌ 拒绝 | 会把 UI / 交互状态耦合进子 agent 运行时语义 |
| 拒绝 | 模糊共享“某些父状态” | ❌ 拒绝 | 没有明确 owner 的共享状态会快速侵蚀调试与回放边界 |

---

## 4. 四层 override 平面

后续所有子 agent 扩展都必须落到下面四层之一，不能再写成“共享一点父状态”这种模糊表述。

### 4.1 Storage

负责事件落点与 session 归属：

- `shared_session`
- `independent_session`

明确约束：

- `storage_mode` 只负责存储落点和 session 归属
- 它**不再暗含**取消传播、任务注册、指标聚合、权限路由等语义

### 4.2 Context Inheritance

负责子执行域看到哪些父上下文：

- system / project instructions
- compact summary
- recent tail
- recovery refs
- parent findings（如果未来开放）

这一层只处理“读到什么”，不处理“能改什么”。

### 4.3 Control Linkage

负责运行时控制链路：

- cancel propagation
- task ownership
- kill routing
- timeout ownership

这一层是 Claude 设计里最值得 Astrcode 吸收的部分，但必须独立建模，不能混入 `storage_mode`。

### 4.4 Observability

负责父侧如何看见子执行域：

- token usage
- step count
- outcome
- findings / artifacts 摘要
- metrics aggregation

这一层是 Astrcode 允许优先扩展的共享面，因为它不会引入父状态直写。

---

## 5. “永不隔离”的控制平面

这是 Astrcode 下一阶段最重要的正式演进方向。

设计结论固定如下：

- 子 agent 启动的 shell / MCP / 长任务注册永远归 runtime 根级 task registry
- `shared_session` 与 `independent_session` 走同一 kill / cleanup / timeout 通道
- 父取消仍然级联子取消
- 任务可观测性与回收责任不挂在 session sharing 语义上

换句话说，**任务 ownership 必须独立于 session ownership**。

这意味着后续实现要审视：

- `turn_ops`
- `agent_execution`
- task 注册路径

并补一层 root-owned task registry / task owner resolver。

这一方向是正式架构演进，不是 experimental。

---

## 6. 先做 shared observability，不做 shared mutable state

Astrcode 允许优先增强的是“父如何看见子”，而不是“子如何直写父”。

允许聚合：

- step
- token
- outcome
- findings
- artifacts 摘要

不允许引入：

- 父 session 的实时可变状态句柄
- 类似 `shareSetAppState` 的共享回调
- 权限提示与 UI 状态直通

`SubRunFinished.result` 继续作为结构化结果中心：

- 父流程消费基于它
- compact 引用基于它
- UI 摘要基于它
- 后续链式 agent 消费也基于它

如果未来需要“子 agent 影响父流程”，只能通过：

- 生命周期事件
- 结构化结果
- 父侧 reducer / coordinator

---

## 7. 优化而非语义：缓存与内容替换共享

以下内容当前被明确降级为“内部优化议题”，不是多 agent 语义模型的一部分：

- file read cache sharing
- content replacement state sharing
- prompt-cache hit 优化

判断标准固定如下：

- 只有当它不改变 replay 语义
- 不改变 debug 语义
- 不改变 compact 语义
- 不需要进入 public DTO

才允许作为 runtime 内部 optimizer / execution cache 进入实现。

在此之前，它们不得进入 `SubagentContextOverrides` 的公开稳定面。

---

## 8. 实施顺序

1. 维持当前“身份模型固定 + 有限 override”的实现方向，不新增自由共享开关
2. 设计并落地 root-owned task registry / task owner resolver
3. 增强 shared observability，但继续禁止父状态直写
4. 仅在行为边界已清晰时，再评估 `independent_session` 的后续扩展
5. 将 file cache / content replacement 共享保留为 runtime 内部优化议题

---

## 9. 对后续实现的硬约束

- 不把 Claude 的 `AppState`、权限提示、拒绝计数共享模型直接搬进 Astrcode
- 不新增“模糊共享某些父状态”的 override 字段
- 不让 `storage_mode` 承担 context / control / observability 的职责
- 不让优化型缓存共享先于 root-owned task control 落地

---

## 10. 一句话结论

Claude Code 给 Astrcode 的真正启发是：**把存储、上下文、控制、观测拆开**。  
Astrcode 应继续走“事件清晰、边界清晰、有限 override”的路线，而不是走“子 agent 可按需直通父运行时状态”的路线。
