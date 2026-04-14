## Context

当前问题的核心不是“child 该不该收缩工具”，而是“这件事应该由谁来表达”。如果让 `AgentProfile` 直接承担工具授权，会出现几个问题：

- profile 变成“角色模板 + 权限组合表”混合体，语义发散；
- `CapabilitySpec` / capability router 不再是唯一能力事实源；
- child 为什么能/不能调用某个工具，很难从 event log 与 status 解释；
- 一旦后续引入 parent→child 继承、task kind bundle、runtime availability，profile 会快速膨胀。

AstrCode 已经有单一 capability semantic model、capability router、policy engine 和 event-log 驱动的 status/read-model。更适合的设计是：profile 继续做行为模板，`spawn` 额外携带任务级 capability grant，runtime 再把它解析成 child 的 resolved capability surface。

## Goals / Non-Goals

**Goals:**

- 保持 `AgentProfile` 为行为模板，而不是权限真相。
- 为 `spawn` 引入任务级 capability grant，并将其解析为 child 的 resolved capability surface。
- 让 child prompt 构建和 tool execution 使用同一份 capability surface。
- 让 subrun 生命周期与状态查询暴露 launch-time capability snapshot，提升可解释性。
- 保持现有 direct-child、mailbox、delivery、wake turn 语义不变。

**Non-Goals:**

- 不实现完整 team / swarm 组织层。
- 不引入跨树通信或 sibling 协作。
- 不在本次内重做 fork/resume 全量上下文继承。
- 不发明新的平行 capability registry。
- 不在第一版就支持复杂的 tag / permission / effect selector DSL。

## Decisions

### 1. `AgentProfile` 保持行为模板，不继续扩大权限语义

`AgentProfile` 在本次变更中继续承担：

- `system_prompt`
- `model_preference`
- 协作/行为默认值

而不承担：

- child 最终工具集合
- launch-time capability 授权真相
- subrun status 的能力解释来源

原因：

- 这符合当前 `CapabilitySpec` 是唯一能力语义模型的架构约束；
- 可以避免 profile 爆炸成权限组合表；
- 行为模板与运行时能力真相的职责边界清楚。

### 2. `spawn` 引入 task-scoped capability grant

在 `SpawnAgentParams` 中引入任务级 capability grant。第一版务实起步，只要求它能表达“本次 child 允许使用的 tool-callable capability names”。

这比直接把工具列表写进 profile 更合适，因为：

- grant 是 task-scoped，而不是 persona-scoped；
- 同一个 `reviewer` profile 可以服务只读 review，也可以服务带 shell 的深度排查，不需要复制 profile；
- launch 时的授权来源更适合写进 event log。

备选方案：

- 在 profile 中继续扩展 `allowed_tools / disallowed_tools`。
  不采用，因为这会让 profile 与 capability router 并列成为权限真相来源。

### 3. child capability 由 resolved surface 求交得到

child 的 resolved capability surface 由以下几项求交得到：

- parent 当前可继承的 capability surface
- spawn capability grant
- runtime availability
- 系统级 capability semantic / policy 护栏

第一版不引入独立 `AgentPolicy` 对象；现有 policy engine 继续负责 `Allow / Deny / Ask`，而 resolved capability surface 只解决“child 看得到什么、可规划到什么、可执行入口有哪些”。

原因：

- 你们已经有 policy engine，不适合在本 change 再造一套并列权限引擎；
- 你们已经有 `CapabilityRouter::subset_for_tools`，可以作为第一版落地点；
- 这样实现成本和概念成本都更低。

### 4. prompt 与 runtime 共用同一份 filtered capability router

launch 后，child prompt 与 runtime 都读取同一份 filtered capability router，而不是分别各算一份工具列表。

这份 router 同时用于：

- prompt facts / prompt build 中的 capability surface
- turn runner 初始化工具列表
- tool execution 前 capability lookup

这样可以保证 prompt、执行与 status 的解释口径一致。

### 5. 复用 `ResolvedExecutionLimitsSnapshot` 承载 launch-time capability snapshot

本次不急着新造 DTO。优先复用并扩展现有 `ResolvedExecutionLimitsSnapshot`，让它承载 child 启动时已经求得的 capability snapshot。

第一版至少包含：

- granted / allowed tool-capable capability names
- `max_steps` 等执行限制

这样 status / replay / debug 都可以直接解释 child 启动时的能力面，而不需要事后重新读取最新配置。

### 6. `spawn` guidance 明确区分 profile 与 capability grant

`spawn` 的指导应明确：

- 选 profile 是在选行为模板；
- 给 capability grant 是在限定本次任务最小能力范围；
- 如果现有 child 的能力面不匹配，再考虑 spawn 新 child；
- 不要把 profile 名称误当成权限组合开关。

## Risks / Trade-offs

- [Risk] profile 未声明工具集时，行为可能与历史默认全量工具混淆
  → Mitigation：本次不继续扩大 profile 工具字段的语义；child 能力真相统一来自 launch-time resolved surface。

- [Risk] filtered capability set 若只在部分链路接线，会产生 prompt/runtime 漂移
  → Mitigation：把 filtered capability view 封装成单一构造入口，prompt 与 runtime 都从该入口取值。

- [Risk] capability grant 第一版若只支持 tool names，会偏工具导向
  → Mitigation：字段命名保持 capability/grant 语义，后续可扩展到 tags / permissions / side-effect selectors。

- [Risk] 显式 capability grant 可能让调用方负担变重
  → Mitigation：grant 保持可选；未显式传入时先采用父级可继承 surface 作为默认路径。

## Migration Plan

1. 增加 execution-side effective profile 解析与 filtered capability view。
2. 在 `spawn` 参数中加入 task-scoped capability grant，并实现 child resolved capability surface 求交逻辑。
3. 将 child prompt assembly 与 turn runner 接到 filtered capability router。
4. 在 child launch / subrun lifecycle 中写入 `ResolvedExecutionLimitsSnapshot`。
5. 扩展 status / observe / debug surface 返回 launch-time capability snapshot。
6. 更新 `spawn` prompt guidance 与相关 specs。

回滚策略：

- 若 capability grant 导致 child 无法完成关键流程，可先保留 snapshot/status 接线，临时退回到“不启用 grant 收缩”的兼容路径；
- 同时修复调用方 grant 使用方式后再重新启用。

## Open Questions

- 第一版 capability grant 是否仅支持 tool-callable capability names，还是同步支持 tags？
- parent→child 的“可继承 capability”是否需要显式区分 inheritable / non-inheritable，还是第一版默认全继承再求交？
- root execution 是否需要在后续复用同一套 resolved capability surface 机制？
