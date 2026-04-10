# Research: 删除死代码与冗余契约收口

## Decision 1: 用“真实消费者 + 明确 owner”定义正式支持面

**Decision**  
本次清理以“当前产品流程是否真的调用它、是否有人能说清它归谁负责”为唯一判定标准。

**Rationale**  
代码库里已经有多类“实现了、测试了、文档写了，但没人真正用”的 surface。继续按“以后可能会用”保留，只会把伪能力永久化。

**Alternatives considered**

- 按“已经实现了就先留着”保留：会让骨架接口永远不会死。
- 按“测试引用到了就算有消费者”保留：会让测试变成死代码的避难所。
- 按“可能存在外部调用方”保留：没有 owner 和 contract 的猜测不足以成为支持理由。

## Decision 2: `SubRunOutcome` 并入 `AgentStatus`，`TokenExceeded` 保留为正式终态

**Decision**  
subrun 终态与 live handle 状态收口到一个 canonical 模型：`AgentStatus`。`TokenExceeded` 作为正式终态加入该模型，并贯穿 core/runtime/server/frontend。

**Rationale**  
当前 `SubRunOutcome` 与 `AgentStatus` 表达的是同一件事，只是词表略有差异（例如 `Aborted` vs `Cancelled`）。继续保留两套模型只会制造映射函数、协议翻译和测试 duplication。

**Alternatives considered**

- 继续保留双模型，再靠 mapper 对齐：会让“状态事实”永久双轨。
- 把 `TokenExceeded` 折叠成 `Completed`：会丢掉真实业务语义。
- 把 `TokenExceeded` 挪到 failure/detail 字段：会让“是否终态”与“为什么终止”重新分裂。

## Decision 3: 删除 `SubRunDescriptor`，让 lineage 事实直接落在 `SubRunHandle`

**Decision**  
删除 `SubRunDescriptor`，把其承载的必要事实直接留在 `SubRunHandle` 与 durable child node 中；`parent_turn_id` 改为必填。

**Rationale**  
当前 descriptor 只是对已经存在的 `sub_run_id` / `parent_turn_id` / `parent_agent_id` / `depth` 的二次封装，还驱动了一批 descriptorless downgrade 分支。去掉 descriptor 后，ownership 与 lineage 的 source of truth 会更直接。

**Alternatives considered**

- 保留 descriptor 作为“durable 专用壳”：会让同一 lineage 事实继续有两种表达。
- 保留 optional `parent_turn_id`：会让 downgrade 语义继续泄漏到主线 contract。
- 在 server/protocol 侧继续拼装 descriptor DTO：会保留一个已经失效的中间层。

## Decision 4: 收口 execution receipt，只保留一个 `ExecutionAccepted`

**Decision**  
`PromptAccepted`、`RootExecutionAccepted` 和 `runtime::service_contract` 中的重复 receipt 收口为一个 `ExecutionAccepted`：`session_id`、`turn_id`、`agent_id: Option<String>`、`branched_from_session_id: Option<String>`。

**Rationale**  
这些类型的差异只来自“由哪个入口返回”，而不是“语义完全不同”。收口后，`core` 与 `runtime` 只需要维护一种 receipt；而旧 root execute HTTP route 又会被删除，不需要额外兼容壳。

**Alternatives considered**

- 继续保留两种 receipt：会让 server/runtime/core 继续维护平行类型。
- 只在 `runtime` 内部统一，`core` 保持分裂：不能真正消除边界间重复。

## Decision 5: `AgentEventContext` 直接从 `SubRunHandle` 投影

**Decision**  
新增 `From<&SubRunHandle> for AgentEventContext`，统一 subrun 事件上下文的构造；保留 `sub_run()` 工厂方法给非 handle 场景复用。

**Rationale**  
当前多个调用点在重复从 `SubRunHandle` 拆字段、拼 `AgentEventContext`。这不是业务差异，只是机械重复，容易漏字段。

**Alternatives considered**

- 全部继续手工构造：重复且容易漂移。
- 删除 `sub_run()` 工厂，只保留 `From`：会让没有 handle 的构造场景变差。

## Decision 6: `launch_subagent` 应归属 live control 边界

**Decision**  
把 `launch_subagent` 从 `ExecutionOrchestrationBoundary` 迁到 `LiveSubRunControlBoundary`。

**Rationale**  
root prompt 提交与 root execute 属于 orchestration；`launch_subagent` 则依赖现有 tool context、live child ownership 和 active control tree，更接近 subrun control 平面。把它挂在 orchestration trait 上会稀释边界职责。

**Alternatives considered**

- 继续留在 orchestration trait：owner 不清晰，调用方语义也变得模糊。
- 为它单独新建第三个 trait：没有必要，会再引入一个 owner。

## Decision 7: `ChildAgentRef` 只保留身份事实，不再承载 UI 派生字段

**Decision**  
`ChildAgentRef` 收口为 agent/session/subrun/lineage/status 的事实模型，不再携带 `openable` 这类 UI 派生值。child navigation 由显式 open target 或 child session durable fact 提供。

**Rationale**  
`openable` 本质上是“当前有没有可打开目标”的派生布尔值，不属于 core 领域事实。把它放在 `ChildAgentRef` 里，会让领域模型背着前端方便字段前进。

**Alternatives considered**

- 在 core 删除、protocol 继续保留同名字段：还是会把兼容壳延续到 transport。
- 保留 `openable` 但加注释：归属问题不会因为注释变好。
- 让前端继续依赖 summary projection route：这会保活已经要删除的 surface。

## Decision 8: 删除无人消费的 summary projection，只保留真实摘要事实

**Decision**  
删除 `loadParentChildSummaryList`、`loadChildSessionView`、`buildParentSummaryProjection` 及对应 server route；保留 `SubRunHandoff.summary` 与 `ChildSessionNotification.summary`。

**Rationale**  
当前 UI 已经通过现有消息流、child notification 和直接打开子会话完成浏览，不需要额外的 parent-summary API / projection。

**Alternatives considered**

- 保留这些 projection，等以后再接：典型预实现。
- 删掉所有 `summary`：会误删主线事实。
- 只删前端不删后端：会留下 server orphan surface。

## Decision 9: `cancelSubRun` 必须先迁移到 `closeAgent`

**Decision**  
`cancelSubRun` 不是立即删除项。先把当前 UI 动作迁到 `closeAgent`，再删除 `/api/v1/sessions/{id}/subruns/{sub_run_id}/cancel` 及其前端包装。

**Rationale**  
当前“取消子会话”按钮仍是活跃主线。直接删除 route 会打断用户功能，不符合“干净收口但不留主线缺口”的目标。

**Alternatives considered**

- 直接删 legacy cancel route：会破坏主线。
- 长期保留 REST cancel + `closeAgent`：会形成双轨主线。
- 为迁移再加一个 adapter route：只是新兼容层。

## Decision 10: 删除无人消费的 public HTTP surface，不保留“未来入口”

**Decision**  
删除 `/api/v1/agents*`、`/api/v1/tools*`、`/api/runtime/plugins*`、`/api/config/reload` 以及对应 protocol/tests/docs。

**Rationale**  
这些 surface 的共同问题不是“实现是否完整”，而是没有当前消费者、没有产品语义、没有 owner。继续暴露只会制造一个不存在的 operator/API 面。

**Alternatives considered**

- 标成 experimental 继续保留：标签不能让无人消费的 surface 变合理。
- 只删 execute、保留 list/status：没有 owner 的半截 surface 没有意义。

## Decision 11: legacy downgrade 统一改为明确失败

**Decision**  
`legacyDurable`、descriptorless subrun 读模型、旧共享历史 downgrade tree 和相关 protocol/frontend/runtime 分支统一退出主线。旧输入进入主线流程时明确失败。

**Rationale**  
如果系统已经决定不支持旧输入，那最清楚的行为就是失败，而不是继续返回“部分可用”的半视图。

**Alternatives considered**

- 继续保留 `legacyDurable`：会把兼容逻辑永久固化。
- 吞掉错误并返回空数据：会掩盖根因。
- 额外构建升级桥接：本次目标是删冗余，不是新增迁移系统。

## Decision 12: live 文档与测试只证明当前主线和明确失败

**Decision**  
更新 `docs/spec/*` 与当前 feature 文档；删除或改写只为旧入口存在感服务的测试。archive 材料可保留，但不再被 live 文档当成现状引用。

**Rationale**  
代码删了而文档/测试不删，仓库会继续向人和自动化工具宣传错误事实。

**Alternatives considered**

- 留着旧文档“作参考”：live 文档会重新污染实现边界。
- 留着旧测试“以防回归”：回归目标会被定义错。

## Decision 13: 为 `action.rs` 的隐式契约补上为什么注释

**Decision**  
在 `ToolExecutionResult::model_content()` 与 `split_assistant_content()` 上补充中文注释，明确说明为什么不能绕过这些入口，以及为什么 `to_ascii_lowercase()` 的字节索引是安全的。

**Rationale**  
这两处是典型“编译器不报错，但稍后一定有人踩坑”的隐式合同。用短注释把原因写清楚，能减少未来把 contract 误删或误改的概率。

**Alternatives considered**

- 不写注释，只靠 review 记忆：维护成本太高。
- 写成长文档：离代码太远，实际没人看。
