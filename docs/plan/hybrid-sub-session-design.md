# 混合子会话 / 受控子会话计划同步

> **最后更新**：2026-04-07  
> **当前结论**：仓库已经从早期的 `isolated_session + ChildSessionSummary` 设想，演进到 **`spawnAgent + controlled sub-session`** 主线。  
> 本文档不再把原始方案当作当前实施蓝图，而是同步：
> 1. 已经落地的真实能力  
> 2. 与旧方案的差异  
> 3. 后续仍值得推进的阶段

---

## 已完成内容（简化总结，放最上面）

### 1. 核心身份模型已经从“事件打标”演进到受控子会话

当前实现已经不只是“子 Agent 事件写回父 session 再打标签”，而是有了稳定的子执行域模型：

- 已有 `sub_run_id`，显式区分 **agent 实例** 与 **子执行域实例**
- 已有 `InvocationKind`，区分 `RootExecution` / `SubRun`
- 已有 `SubRunStorageMode::{SharedSession, IndependentSession}`
- 已有 `SubRunHandle.child_session_id`
- 已有 `AgentEventContext.child_session_id`
- 已有 `SubRunStarted / SubRunFinished` 生命周期事件

**对应实现：**
- `crates/core/src/agent/mod.rs`
- `crates/core/src/event/types.rs`

### 2. Runtime 已形成统一的子会话执行链

当前 `spawnAgent` 的运行时路径已经比较稳定，不再需要早期文档里那套“分支式伪代码”来解释：

- `launch_subagent()` 是统一入口
- 已拆出：
  - `resolve_profile()`
  - `resolve_parent_execution()`
  - `prepare_child()`
  - `spawn_child()`
  - `build_event_sinks()`
  - `run_child_loop()`
  - `finalize_child_execution()`
- `IndependentSession` 模式下会创建 child session，并使用独立 sink
- `SharedSession` 模式下继续复用父 session sink
- 后台子会话返回结构化 `subRun` artifact；如果是独立子会话，还会返回 `session` artifact

**对应实现：**
- `crates/runtime/src/service/execution/subagent.rs`
- `crates/runtime-execution/src/prep.rs`

### 3. API / 前端 / 测试已经具备可用骨架

**API 已落地：**
- `GET /api/v1/agents`
- `POST /api/v1/agents/{id}/execute`
- `GET /api/v1/sessions/{id}/subruns/{sub_run_id}`
- `POST /api/v1/sessions/{id}/subruns/{sub_run_id}/cancel`

**前端已落地：**
- 子会话事件已进入状态层
- `MessageList` 已按 `subRunId` 归组
- `SubRunBlock` 已能内联展示运行中 / 完成 / 失败 / token 超限等状态

**测试已落地：**
- 子会话事件链基本集成测试
- 后台 `subRun` artifact 测试
- 显式取消释放并发槽位测试

**对应实现：**
- `crates/server/src/http/routes/mod.rs`
- `crates/server/src/http/routes/agents.rs`
- `frontend/src/components/Chat/SubRunBlock.tsx`
- `frontend/src/lib/applyAgentEvent.ts`
- `crates/runtime/src/service/execution/tests.rs`

---

## 与原始方案的主要差异（需要明确）

这部分是同步文档时最重要的地方：**哪些内容已经被主线吸收，哪些内容已经偏离实际，不应再当作短期实施目标。**

### 1. `isolated_session` 没有作为 `spawnAgent` 的公开参数落地

原方案希望：

```rust
pub struct SpawnAgentParams {
    pub isolated_session: bool,
}
```

但当前真实实现没有走这条路。现在的设计是：

- `spawnAgent` 工具面保持极简，不暴露额外 override
- `storage_mode` / `independent_session` 主要留在 runtime / API 的受控边界内
- 避免把更多存储语义开关直接暴露给 LLM

### 2. 没有采用 `ChildSessionSummary`，而是采用 `SubRunStarted / SubRunFinished`

原方案希望父 session 写入一个独立的 `ChildSessionSummary` 事件。  
当前实现没有采用该事件，而是使用：

- `SubRunStarted`
- `SubRunFinished { result, step_count, estimated_tokens }`

这使得父侧消费统一基于：

- 生命周期事件
- `SubRunFinished.result`
- `handoff.summary / findings / artifacts`

而不是再新增一套平行摘要事件。

### 3. “点击卡片跳转到完整子会话页面”还没有产品化落地

原方案里前端目标是：

- 父会话展示 `ChildSessionSummary` 卡片
- 点击后跳转到子会话详情页

当前实现更保守：

- 使用 inline `SubRunBlock`
- 优先保证可观测性与稳定性
- `childSessionId` 已贯通，但还没有完整变成“独立子会话详情页”产品交互

### 4. `IndependentSession` 已可工作，但仍保持 experimental

这点必须在文档中写清楚：

- `SharedSession` 是正式路径
- `IndependentSession` 是实验路径
- 当前承诺是“可查询 / 可展示 / 可回填结果”
- 在控制平面和观测平面完全清晰前，不扩大产品承诺范围

参考：
- `docs/design/subagent-session-modes-analysis.md`

---

## 当前主线设计（基于真实实现）

### 核心结论

当前更准确的表述应该是：

> **默认主线是受控子会话（controlled sub-session），而不是“共享 session / 独立 session 二元切换工具参数”。**

其中：

- `SharedSession` / `IndependentSession` 是 **存储落点**
- `SubRunStarted / SubRunFinished` 是 **生命周期事件**
- `SubRunResult` 是 **父流程消费的结构化结果中心**
- `child_session_id` 是 **独立子会话的引用**

### 当前推荐的数据消费方式

父流程与 UI 不应依赖“额外摘要事件”，而应优先消费：

- `SubRunFinished.result.summary`
- `SubRunFinished.result.findings`
- `SubRunFinished.result.artifacts`
- `SubRunFinished.step_count`
- `SubRunFinished.estimated_tokens`

这与当前 runtime / server / frontend 的真实实现是一致的。

---

## 后续阶段（按当前主线整理）

## Phase 1：文档与命名收口

### 目标

把历史文档统一到真实实现，避免后续讨论继续混用两套语义。

### 需要完成

- [ ] 将涉及 `isolated_session` 的旧计划标记为“历史设想 / 未采纳”
- [ ] 将“子会话摘要事件”统一改写为 `SubRunStarted / SubRunFinished`
- [ ] 将示例里的 `ChildSessionSummary` 流程改写为当前事件流
- [ ] 在相关文档中统一使用：
  - `spawnAgent`
  - `SubRun`
  - `SharedSession / IndependentSession`
  - `controlled sub-session`

### 如何完成（思考）

这一步优先级最高，因为现在的主要问题不是代码缺失，而是**文档仍然在描述已经偏离的实施路线**。  
如果不先收口，后续所有设计讨论都会重复落到：

- 到底是继续做 `isolated_session` 参数，还是保持当前工具面极简？
- 到底是再加 `ChildSessionSummary`，还是沿用 `SubRunFinished.result`？

### 建议

- **不要把 `isolated_session` 重新加回工具参数。**
- **不要再新增平行的摘要事件类型，优先复用 `SubRunFinished.result`。**

---

## Phase 2：控制平面补强

### 目标

把“子任务归谁管、如何 kill / cleanup / timeout”从存储语义中剥离出来。

### 需要完成

- [ ] 设计并落地 `root-owned task registry`
- [ ] 补 `task owner resolver`
- [ ] 统一 `SharedSession` / `IndependentSession` 的 kill / cleanup / timeout 通道
- [ ] 明确长任务、shell、MCP 的 owner 与回收责任

### 如何完成（思考）

下一阶段最重要的不是继续增加 session 相关开关，而是解决：

1. **任务 ownership**
2. **控制链路一致性**

也就是：

- session 归属是谁
- task 归属是谁

必须拆开。

### 建议

- **先做 root-owned task control，再考虑扩大 `IndependentSession` 的产品承诺。**
- **不要让 `storage_mode` 承担取消、任务注册、指标聚合职责。**

---

## Phase 3：观测与结果聚合补强

### 目标

增强父流程“如何看见子流程”，而不是让子流程直接改父状态。

### 需要完成

- [ ] 强化 step / token / outcome 聚合
- [ ] 规范 findings / artifacts 的聚合展示
- [ ] 让 `SubRunFinished.result` 更稳定地承担父侧 handoff 中心
- [ ] 明确 `IndependentSession` 在 replay / compact / debug 下的边界

### 如何完成（思考）

仓库主线已经明确拒绝“父状态直写”。  
因此后续增强应该集中在：

- 生命周期事件
- 结构化结果
- 父侧 reducer / coordinator

而不是：

- 子流程直接共享父可变状态

### 建议

- **优先增强 observability，不要引入 shared mutable state。**
- **如果未来需要“子影响父”，也只能通过结果与事件，而不是回调句柄。**

---

## Phase 4：Storage / API 能力按需补齐

### 目标

仅在产品和前端确实需要时，再补父子会话查询接口，而不是为了匹配旧设计而补。

### 需要完成

- [ ] 评估是否真的需要 `list_child_sessions(parent_session_id)`
- [ ] 如果需要，再补 repository / server 查询接口
- [ ] 评估是否要新增：
  - `GET /api/v1/sessions/{parent_id}/child-sessions`
  - 或基于现有 session 查询体系复用
- [ ] 统一 child session 详情的获取方式，避免另起一套平行 API

### 如何完成（思考）

原始文档里的：

- `list_child_sessions()`
- `GET /api/sessions/{parent_id}/child_sessions`
- `GET /api/sessions/{child_id}`

这些接口都不是当前最短路径。  
更合理的做法是先判断：

- 前端是否真的需要“父 → 子会话列表页”
- 还是只需要一个“打开该 child session”的能力

### 建议

- **先补最小可用查询能力，不要一次性扩成完整父子会话 API 套件。**
- **尽量复用既有 session 查询接口，而不是新增过多专用路由。**

---

## Phase 5：前端从“能看”升级到“更好用”

### 目标

在不破坏当前 inline 可观测性的前提下，让独立子会话更易读、更易打开。

### 需要完成

- [ ] 优化 `SubRunBlock` 的信息层级
- [ ] 对存在 `childSessionId` 的独立子会话增加“打开子会话”入口
- [ ] 评估是否需要“完整子会话详情页”
- [ ] 明确父 / 子会话切换后的返回路径

### 如何完成（思考）

当前前端已经证明协议链路是通的，所以下一步重点不应是再造协议，而应是：

- 降低阅读成本
- 改善运行中与完成态呈现
- 让 `childSessionId` 真正变成可操作引用

### 建议

- **短期继续保留 inline `SubRunBlock`。**
- **中期只在 `IndependentSession` 情况下增加“打开子会话”按钮。**
- **不要急着做复杂树状多层 UI，先把单层 subrun 体验做好。**

---

## Phase 6：测试补强

### 目标

守住最容易在重构中回退的边界。

### 需要完成

- [ ] 增加 `IndependentSession` 端到端测试：
  - child session 建立
  - parent / child sink 分离
  - `child_session_id` 回填
  - 查询 / 取消 / 展示链路
- [ ] 补 `SubAgentPolicyEngine` 相关边界测试
- [ ] 补 `CapabilityRouter::subset_for_tools` 测试
- [ ] 覆盖 `SubRunStarted / SubRunFinished` 的生命周期完整性

### 如何完成（思考）

这里不应只补 happy path，而应优先守住：

- 参数校验
- 工具裁剪
- 存储切换
- 后台取消
- 生命周期事件完整性

### 建议

- **测试优先围绕“边界”和“回退风险”来写，而不是继续堆功能展示型测试。**

---

## 当前不建议作为短期目标推进的旧项

以下内容建议在短期内明确标记为“不按当前主线推进”：

- `spawnAgent.isolated_session: bool` 作为公开工具参数
- `ChildSessionSummary` 作为新的父侧摘要事件
- 为了匹配旧文档而强行补一套专用父子会话 API
- 先做复杂子会话页面导航，再回头补控制平面

**原因**：这些内容要么已经被当前实现替代，要么实现顺序不合理，容易导致语义再次分叉。

---

## 推荐执行顺序

1. **先改文档**：统一到真实主线
2. **再补控制面**：task registry / owner resolver
3. **再补 observability**：继续围绕 `SubRunFinished.result`
4. **再做前端增强**：优先改善 inline block 与 child session 打开能力
5. **最后补专用 API 与更多 UI**：仅在需求被证明后再做

---

## 相关文档

- [Agent as Tool + 开放 API 实施计划](./agent-tool-api-implementation-plan.md)
- [多 Agent 会话模式：对 Claude 设计的采纳与边界](../design/subagent-session-modes-analysis.md)
- [当前设计文档](../design/agent-tool-and-api-design.md)
- [Agent Loop 内容架构](./agent-loop-content-architecture.md)
