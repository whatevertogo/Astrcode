# Findings: 当前子 Agent / SubRun / Child Session 现状

## 1. `spawnAgent` 仍是“只负责后台启动”的单一工具

当前 `crates/runtime-agent-tool/src/spawn_tool.rs` 只实现了 `spawnAgent`，并在工具说明里明确写死“默认异步、统一用后台子会话方式启动”。  
它只负责：

- 参数反序列化和校验
- 调用 `SubAgentExecutor::launch`
- 把 `SubRunResult` 投影成 tool 结果

这意味着现有工具面没有 `send / wait / close / resume / deliver` 这些协作能力，主子协作闭环并未形成。

## 2. 运行时子执行链路仍以 `SubRunHandle + parent_turn_id` 为中心

`crates/runtime/src/service/execution/subagent.rs` 当前的 `launch_subagent()` 路径要求父 `turn_id` 存在，随后：

- 解析 profile
- 解析父执行上下文
- 构造 child handle 和 child turn id
- 立即返回 `SubRunOutcome::Running`
- 后台 `tokio::spawn` 执行 child loop

这里还没有显式的 parent/child inbox、单次消费语义或“把 child 交付重新送入 parent agent 输入流”的抽象，更多仍是 spawn 后异步跑完再收口。

## 3. live 控制面只有 `get/cancel`，没有持续协作控制面

当前 `LiveSubRunControlBoundary` 在 `crates/runtime/src/service/execution/mod.rs` 只暴露：

- `get_subrun_handle`
- `cancel_subrun`
- `list_profiles`

这说明 runtime 目前具备“查子执行”和“取消子执行”的能力，但没有“向特定子 agent 发送追加要求”“等待某个子 agent”“恢复旧 child agent”“由 child 向 parent 受控交付”这套控制面。

## 4. server / frontend 当前仍围绕 subrun status + cancel，而不是独立 child session

当前 server 的公开 subrun surface 主要是：

- `GET /api/v1/sessions/{id}/subruns/{sub_run_id}`
- `POST /api/v1/sessions/{id}/subruns/{sub_run_id}/cancel`

对应前端 `frontend/src/lib/api/sessions.ts` 也只有 `cancelSubRun()`；`useAgent.ts` 和 `App.tsx` 仍通过 `loadSession(sessionId, filter)` 与 `buildSubRunThreadTree()` 从父会话混合消息中构造 child 浏览模型。

这表明“独立 child session 可直接打开”在 durable truth 上还没有真正成为前端主模型。

## 5. 前端 `SubRunThreadTree` 依赖 mixed-session 假设

`frontend/src/lib/subRunView.ts` 当前会把父会话里带 `subRunId` 的消息聚合成 `SubRunRecord`，再用：

- `descriptor.parentAgentId`
- `agentOwnerMap`
- `ownBodyEntries + directChildSubRunIds`

生成一个 thread tree。  
这个模型适合“父会话里混着子执行消息”的场景，不适合“child 有自己完整 session，parent 只保留通知投影”的模型。

## 6. 独立 child session 的物理存储已经存在，但 ownership 语义还未收口

`storage` 侧现在已经按 `sessions/<session-id>/session-<session-id>.jsonl` 管理单个 session 目录，`SubRunStorageMode::IndependentSession` 也允许 child 落到独立 session。  
但当前 durable 真相仍主要围绕 `sub_run_id` 与 mixed event 组合工作，parent/child 所有权、resume/fork lineage、session-level child navigation 还不是第一等 durable 模型。

## 7. registry 层仍保留 tool/capability 双轨，未来协作工具扩展会被放大

`runtime-registry` 里：

- `CapabilityRouter` 已经是生产执行主入口
- `ToolRegistry` 仍保留 tool → capability 转换与测试装配能力
- `capability_context_from_tool_context()` 仍集中注入 runtime 默认 profile/context

当协作工具从 `spawnAgent` 扩展成一组工具族后，这种双轨和上下文默认值注入会更容易成为边界污染点，因此必须在本轮计划里一起收口。

---

## 实现状态（2026-04-09 更新）

以上 7 项 findings 均已在当前实现中解决：

| Finding | 解决方式 |
|---------|---------|
| F1: `spawnAgent` 单一工具 | 新增 `sendAgent`、`waitAgent`、`closeAgent`、`resumeAgent`、`deliverToParent` 五个协作工具 |
| F2: 以 `SubRunHandle + parent_turn_id` 为中心 | 引入 `ChildSessionNode` 作为 durable 真相，child session 独立于父 turn 存活 |
| F3: 只有 `get/cancel` 控制面 | 完整协作控制面：send/wait/close/resume/deliver，按 ownership subtree 级联 |
| F4: server/frontend 围绕 subrun status | 新增 `loadParentChildSummaryList`、`loadChildSessionView` API，父摘要 + 子直开模型 |
| F5: `SubRunThreadTree` 混合假设 | 新增 `buildParentSummaryProjection` 直接从索引构建摘要卡片，legacy tree 降级为旧版兼容 |
| F6: ownership 语义未收口 | `ChildSessionNode` 持有完整 ownership 链，`lineage_kind` 区分 spawn/fork/resume |
| F7: registry 双轨 | `ToolRegistry` 退化为纯测试辅助，`CapabilityRouter` 成为唯一生产入口 |
