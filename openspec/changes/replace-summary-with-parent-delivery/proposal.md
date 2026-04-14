## Why

当前子代理回流父级的主语义仍然建立在 `summary / final_reply_excerpt / server summary projection` 这组字段上，而不是建立在正式的父子消息合同上。结果是同一份 child 结果会在 runtime、server mapper、前端父视图里被重复投影，既让 UI 层级变怪，也让“child 是否真的向 parent 交付了一条消息”变得模糊。

Astrcode 现有架构已经明确了 `parent -> child send`、direct-parent 所有权、durable delivery 与 parent wake 的边界；现在缺的不是更多 summary，而是把 `child -> parent` 也提升为正式消息语义，并把 summary 从 server / frontend 的主合同里删掉。

## What Changes

- 新增 child-scoped 的显式上行交付语义，允许 child 通过正式业务入口向 direct parent 发送进度、完成、失败与关闭申请消息，而不是依赖 `summary` 被动投影。
- 修改 child terminal finalizer：当 child 本轮已经显式向 parent 交付完成态消息时，不再额外制造一份 summary-based terminal 文本；若 child 直接进入 terminal/idle 且没有显式上报，则 runtime 走确定性的 fallback delivery。
- 修改 parent wake 合同：wake prompt 与 durable parent delivery 统一消费正式消息内容，不再依赖 `summary + final_reply_excerpt` 的双轨文本模型。
- 删除 server / protocol / frontend 中围绕 child handoff 和 child notification 的 summary 主字段，让父视图改为消费 typed parent-delivery message 与 child session 入口，而不是 server 生成的 summary 卡片。
- 修改 child prompt / tool guidance：child 必须在阶段性进展或任务结束时通过上行消息向 parent 汇报；若任务完成且责任域结束，可一并申请被 close。
- **BREAKING** 移除 `summary` 作为 subagent handoff、child notification 与 server child summary projection 的正式对外语义；相关 HTTP/SSE DTO、前端事件与测试夹具需要同步更新。

## Capabilities

### New Capabilities

- 无

### Modified Capabilities

- `agent-delivery-contracts`: child delivery 从 summary-based projection 改为 typed parent-delivery message，direct-parent 上行交付、wake 消费与 idle fallback 的 requirement 需要更新。
- `subagent-execution`: 子代理执行合同需要新增 child→parent 显式回消息义务、关闭申请语义，以及“terminal/idle 但未显式上报”时的 deterministic fallback。
- `agent-tool-governance`: child-scoped prompt guidance 需要从“结束时给出 summary”改为“通过正式上行消息回复 parent”，并明确 `send` 与 child→parent reply 不是同一个动作。

## Impact

- 影响代码：
  - `crates/application/src/agent/{mod,routing,terminal,wake}.rs`
  - `crates/adapter-tools/src/agent_tools/*`
  - `crates/server/src/http/{mapper,routes}/*`
  - `crates/protocol/src/http/*`
  - `frontend/src/{types,lib,components}/**/*`
- 影响运行时语义：
  - child completion / failure / close 不再以 summary 为正式交付主语义
  - parent wake 改为消费 typed delivery message
  - child terminal idle fallback 变成正式兜底机制
- 影响用户可见行为：
  - 父视图看到的是“子 Agent 发来的消息 / 关闭申请 / 子会话入口”，而不是 server 合成的 summary 文本
  - child 若没有主动回 parent，系统仍会在 terminal 时自动补一条 fallback delivery，避免结果丢失
- 影响开发者可见行为：
  - server 不再维护 child summary projection
  - protocol / frontend 需要围绕 typed parent delivery 重写事件投影与展示逻辑
  - prompt contract 必须把 child→parent 回复动作写成正式协议，而不是依赖自然语言约定

## Non-Goals

- 不把现有 `send` 改造成双向泛化消息工具；`parent -> child` 与 `child -> parent` 保持不同语义与入口。
- 不引入“child idle 后再额外问一轮 LLM 是否完成任务”的追问状态机。
- 不把跨 session wake 编排下沉到 `session-runtime` 或 `kernel`。
- 不在本 change 中重做整个 debug/workbench summary 模型；只处理 child-parent collaboration 主链和 server/frontend 正式合同。

## Migration And Rollback

- 迁移方式采用“一次性切换主合同”：
  1. 先引入 typed parent-delivery message 与 child 上行回复入口；
  2. 再让 wake / server / frontend 全部切到新字段；
  3. 最后删除旧 `summary` 相关 mapper、DTO 与父视图投影。
- 由于本仓库不追求向后兼容，本次允许直接 breaking change，但必须保证 server、frontend、测试夹具在同一 change 内一起完成迁移。
- 若集成阶段发现 child 显式回复链路不稳定，可临时只保留“typed fallback delivery + parent wake”，但不得恢复 server-side summary projection；回滚时应整体恢复旧 DTO、旧 wake 文本与旧父视图投影，而不是混用新旧合同。
