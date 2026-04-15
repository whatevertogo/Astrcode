## Why

当前 child 结果回流父级仍然主要依赖 `summary / final_reply_excerpt / server summary projection`。真正的业务事实其实是“这个 child 向 direct parent 交付了一条什么消息”，但现状却把它拆成了 terminal summary、server mapper projection 和前端 summary card 三层弱语义，导致：

- 父视图展示的是 server 生成的摘要，而不是 child 的正式消息
- child 是否真的向 parent 交付了一条消息不够清晰
- 子会话完成后，parent wake 很难只围绕正式消息继续协作

仓库现有实现里，`send` 已经是父子协作主入口，`application` 也已经围绕 `send -> mailbox -> wake/finalizer` 建好了主链。最天然的做法不是新增一个 `reply_to_parent` 工具，而是把 `send` 统一成一个同名、分型、方向感知的协作入口：

- `parent -> child` 时，`send` 继续发送下一条具体指令
- `child -> direct parent` 时，`send` 发送 typed upward delivery

这样既能删掉 summary 主合同，也不会再多造一套 capability surface。

## What Changes

- 把 child -> parent 回流提升成 typed parent-delivery message，并通过统一 `send` 工具发送，而不是依赖 `summary` 被动投影。
- 修改 `send` 合同：同一个工具名下根据这次协作消息的方向分流为下行消息或上行 typed delivery；中间层 agent 既能向上回 parent，也能向下发 child，不新增独立 `reply_to_parent` 工具。
- 修改 child terminal finalizer：如果当前 child work turn 已经通过 unified `send` 显式发出 terminal upward delivery，则不再额外制造 fallback；否则仍走 deterministic fallback。
- 修改 parent wake 合同：wake prompt 与 durable parent delivery 统一消费 typed delivery，不再依赖 `summary + final_reply_excerpt`。
- 删除 server / protocol / frontend 中围绕 child handoff 和 child notification 的 summary 主字段，让父视图改为消费 typed parent-delivery message 与 child session 入口。
- 修改 child prompt / governance：child 必须在阶段性进展或任务结束时通过 unified `send` 向 direct parent 汇报；若任务完成且责任域结束，可一并发送 `close_request`。
- **BREAKING** 移除 `summary` 作为 subagent handoff、child notification 与 server child summary projection 的正式对外语义；相关 HTTP/SSE DTO、前端事件与测试夹具需要同步更新。

## Capabilities

### Modified Capabilities

- `agent-delivery-contracts`: child delivery 从 summary-based projection 改为 typed parent-delivery message，并明确 unified `send` 在 child 上下文中承担上行职责。
- `subagent-execution`: 子代理执行合同需要新增 child 使用 unified `send` 上行回 parent 的义务，以及 terminal/idle 未显式上报时的 deterministic fallback。
- `agent-tool-governance`: prompt guidance 需要从“结束时给出 summary”改为“通过 unified `send` 沿协作树上行或下行发正式消息”，并明确中间层 agent 可以同时使用两种方向。

## Impact

- 影响代码：
  - `crates/application/src/agent/{mod,routing,terminal,wake}.rs`
  - `crates/adapter-tools/src/agent_tools/*`
  - `crates/server/src/http/{mapper,routes}/*`
  - `crates/protocol/src/http/*`
  - `frontend/src/{types,lib,components}/**/*`
- 影响运行时语义：
  - child completion / failure / close 不再以 summary 为正式交付主语义
  - `send` 变成统一协作消息入口，但 routing / payload 仍严格分型
  - child terminal idle fallback 变成正式兜底机制
- 影响用户可见行为：
  - 父视图看到的是“子 Agent 发来的消息 / 关闭申请 / 子会话入口”，而不是 server 合成的 summary 文本
  - child 若没有主动回 parent，系统仍会在 terminal 时自动补一条 fallback delivery，避免结果丢失
- 影响开发者可见行为：
  - server 不再维护 child summary projection
  - protocol / frontend 需要围绕 typed parent delivery 重写事件投影与展示逻辑
- `send` 的工具 schema 与 prompt 需要表达 upstream/downstream 两种参数分支，并允许中间层 agent 在同一轮同时使用两种方向

## Non-Goals

- 不新增独立 `reply_to_parent` 工具。
- 不把 unified `send` 做成松散的双向聊天入口；上下行 payload、ownership 和 routing 仍必须严格区分。
- 不引入“child idle 后再额外问一轮 LLM 是否完成任务”的追问状态机。
- 不把跨 session wake 编排下沉到 `session-runtime` 或 `kernel`。
- 不在本 change 中重做整个 debug/workbench summary 模型；只处理 child-parent collaboration 主链和 server/frontend 正式合同。

## Migration And Rollback

- 迁移方式采用“一次性切换主合同”：
  1. 先引入 typed parent-delivery message，并把 unified `send` 扩展为方向感知入口；
  2. 再让 wake / server / frontend 全部切到新字段；
  3. 最后删除旧 `summary` 相关 mapper、DTO 与父视图投影。
- 由于本仓库不追求向后兼容，本次允许直接 breaking change，但必须保证 server、frontend、测试夹具在同一 change 内一起完成迁移。
- 若集成阶段发现 child 上行链路不稳定，可临时只保留“typed fallback delivery + parent wake”，但不得恢复 server-side summary projection；回滚时应整体恢复旧 DTO、旧 wake 文本与旧父视图投影，而不是混用新旧合同。
