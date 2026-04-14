## Context

Astrcode 当前已经完成了两项关键铺垫：

- 子代理最大深度和每轮 fan-out 已经具备 runtime 硬限制。
- child terminal delivery 与 parent wake 的主链路已经被收口到稳定编排模型。

但这两项基础设施只解决了“系统能不能约束”和“消息能不能送达”，还没有解决“模型应该如何正确使用四个协作工具”。结果就是：

- `spawn` 仍然容易被当成默认探索动作，而不是昂贵的委派动作。
- `observe` 仍然更像原始状态快照，而不是面向下一步决策的查询。
- `send` 与 `close` 的使用边界主要靠工具文案暗示，缺少系统级协议。
- child session 在持久化层是独立 session，但在交互心智上其实是“父代理的一个任务分支”，两者之间还缺少正式桥接。

Claude Code 的经验表明，真正降低噪音的不是让模型知道更多内部实现，而是让它更清楚“下一步该用哪个工具”。因此本次设计的重点是借鉴它的协作协议和任务思维，而不是照搬它的 runtime 形态。

## Goals / Non-Goals

**Goals:**

- 在不改变当前子代理编排架构的前提下，定义一套正式的四工具协作协议。
- 将协作规则分层到系统 guidance、工具 prompt 和运行时结果投影，减少重复和漂移。
- 让 `observe`、`send`、`close` 的结果更利于模型快速做出下一步决策。
- 保持 child session 的事件持久化与恢复语义，同时让 UI/交互心智更接近“任务分支”。

**Non-Goals:**

- 不引入新的 fork runtime、worktree teammate 或跨 session teammate mailbox 模型。
- 不用提示词替代 runtime 限制；已有深度和 fan-out 限制继续保留。
- 不在本 change 中实现指标采集、效果评分或实验看板。

## Decisions

### 决策 1：借鉴 Claude Code 的“协作协议”，不照搬它的“进程模型”

Astrcode 已经选择了 child session + direct-parent mailbox 的架构，这套模型和事件日志、恢复语义是相容的。Claude Code 的强项在于：

- 把子 agent 看成 task，而不是普通聊天分叉。
- 把 `Agent` / `SendMessage` 的使用约束写成明确协议。
- 通过进度、summary、notification 把主线程从“盯状态”里解放出来。

本次只借鉴这些设计思想，不引入 fork/self-fork 或外部 teammate runtime。  
替代方案是完整迁移到 Claude Code 风格的 fork/task runtime，但这会同时改动 session 模型、恢复语义和前端展示层，超出本次范围。

### 决策 2：把协作治理拆成三层，而不是把所有规则都塞进工具描述

三层分别承担不同职责：

- 系统级 collaboration guide：定义统一状态机和默认行为，如“Idle 是正常可复用状态”“没有明确独立收益时不要继续 spawn”“observe 必须服务于决策”。
- 工具级 prompt metadata：只说明该工具的单一职责和最小约束，避免长篇理念重复注入上下文。
- 运行时结果投影：为 `observe` 等工具补足决策友好的结构化结果，而不是只暴露底层状态。

替代方案一是把所有规则都写进 `spawn/send/observe/close` 的 description；缺点是重复、上下文噪音大、容易漂移。  
替代方案二是只改 runtime 不改 prompt；缺点是模型仍然不知道该怎样更自然地使用这些结果。

### 决策 3：`observe` 返回“原始状态 + 建议动作”，但建议动作不是新的业务真相

`observe` 需要继续返回当前的 lifecycle、lastTurnOutcome、pendingMessageCount 等事实字段，但还应补一层非权威的决策投影，例如：

- `recommendedNextAction`
- `recommendedReason`
- `deliveryFreshness`

这些字段只用于帮助主代理判断“现在应该 wait、send 跟进还是 close 分支”，不能替代原始事实，也不能变成新的持久化真相。  
替代方案是让模型自己从原始状态推断下一步，但这正是当前产生无意义轮询和重复 spawn 的主要来源。

### 决策 4：`send / observe / close` 的 direct-child 所有权必须成为显式合同

当前实现已经具备 direct-parent 约束，但它更多体现在 runtime 校验里。本次需要把它上升为正式合同：

- 只能对自己直接拥有的 child 使用 `send / observe / close`
- `send` 是向 child mailbox 追加下一条具体指令，不是催促、广播或状态探测
- `close` 是 cascade close 一个任务分支，不是“看一下是不是结束了”
- `observe` 是 direct-child 的同步查询，不是跨树浏览器

这样可以把“谁拥有这个 child、谁有权继续推进它”说清楚，也便于前端和调试工具正确解释 child lineage。

### 决策 5：child session 继续作为持久化事实，但默认交互心智改成“任务分支”

Server truth 和事件回放要求 child 仍然是正式 session，因此本次不改 session 持久化模型。  
但对前端和工具结果来说，应当更明确地表达：

- child session 是某个 parent agent 拥有的执行分支
- 默认不把它当作普通顶层会话使用
- 显式打开 child 时再进入该 transcript / detail view

替代方案是直接把 child 降成纯内存任务对象，但会破坏当前恢复和调试语义。

## Risks / Trade-offs

- [Risk] 协作 guidance 过强，抑制本来合理的并行委派  
  Mitigation：明确保留“独立工作流可并行 spawn”的例外路径，并继续使用 fan-out limit 兜底。

- [Risk] `observe` 的建议动作被模型误当成强制命令  
  Mitigation：spec 和实现都将其标记为 advisory projection，原始 lifecycle/outcome 仍然是唯一业务事实。

- [Risk] 系统级 guidance 与工具级 prompt 重复，后续容易漂移  
  Mitigation：把共享规则收口到 `workflow_examples` / collaboration guide，工具级 prompt 只保留单工具动作约束。

- [Risk] child lineage 展示收紧后，开发者觉得“找不到子会话”  
  Mitigation：保留显式打开、深链跳转和 debug 视图，不删除 child transcript 本身。

## Migration Plan

1. 先补齐 proposal/spec，明确四工具协议与 direct-child 合同。
2. 在 `adapter-prompt` 中收口 collaboration guide，把共享规则从各工具 description 中剥离出来。
3. 在 `adapter-tools` 与 `application::agent` 中增加 decision-oriented 结果投影与更清晰的 send/observe/close 结果语义。
4. 如需要，更新前端默认 child 展示策略，但不改变 session 持久化结构。
5. 回滚时优先撤回新增投影字段与 prompt 治理；direct-parent runtime 校验保持不变。

## Open Questions

- `recommendedNextAction` 一类字段是否应同时出现在 HTTP observe route，还是仅用于 tool result？
- child lineage 的展示收口是否需要一个单独的 frontend capability，还是作为本 change 的附属实现即可？
