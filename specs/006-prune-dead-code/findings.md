# Findings: 删除死代码与冗余契约收口

本文件记录当前仓库里已经确认的事实，用来约束后续实现，不再靠印象判断“到底是不是冗余”。

## Finding 1: 前端没有 React Router，所谓“前端路由死代码”主要是状态读模型与 projection

- 当前导航由 `activeProjectId`、`activeSessionId`、`activeSubRunPath` 驱动。
- `App.tsx`、`sessionView.ts`、`store/reducer.ts` 与 focused subrun filter 构成真实导航主线。

**Implication**  
清理重点不是 router 框架代码，而是没有消费者的 API wrapper、projection、legacy 读模型分支和 duplicated open helper。

## Finding 2: `loadParentChildSummaryList`、`loadChildSessionView` 与 `buildParentSummaryProjection` 当前都没有 UI 消费者

- `frontend/src/lib/api/sessions.ts` 导出了 `loadParentChildSummaryList` 与 `loadChildSessionView`。
- `frontend/src/lib/subRunView.ts` 导出了 `buildParentSummaryProjection`、`ParentSummaryProjection`、`ChildSummaryCard`。
- 当前产品入口里没有调用这些导出；现有引用主要来自测试和文档。

**Implication**  
这组 surface 属于立即删除项，而不是“等以后再接”的预实现。

## Finding 3: `cancelSubRun` 仍是活跃主线，不属于立即删除项

- `SubRunBlock` 的取消按钮当前仍通过 `onCancelSubRun` 触发。
- `Chat`、`App`、`useAgent`、`frontend/src/lib/api/sessions.ts` 仍把动作接到 `/api/v1/sessions/{id}/subruns/{sub_run_id}/cancel`。

**Implication**  
必须先把 UI 切到 `closeAgent`，再删 legacy cancel route。

## Finding 4: 多个 server public surface 有实现、有测试，但没有当前产品入口

- `/api/v1/agents*`、`/api/v1/tools*`、`/api/runtime/plugins*`、`/api/config/reload` 都有 handler 或测试。
- 当前前端没有消费者，也没有明确 operator 面在产品里暴露这些能力。

**Implication**  
“有 handler / 有测试”不能成为保留理由。

## Finding 5: live 文档仍在宣传与主线不一致的能力面

- `docs/spec/agent-tool-and-api-spec.md` 仍把 `/api/v1/agents`、`/api/v1/tools` 等描述成当前 API 面。
- `docs/spec/session-and-subrun-spec.md` 仍在描述旧共享/独立子会话语义的演进阶段。
- `docs/spec/open-items.md` 还在把已决定删除的接口写成待讨论项。

**Implication**  
如果不更新 live 文档，仓库会继续暗示这些 surface 仍然受支持。

## Finding 6: `legacyDurable` 与 descriptorless downgrade 公开语义仍散落在前端、协议和运行时

- `frontend/src/types.ts` 仍公开 `legacyDurable` 和 related unsupported/legacy types。
- `subRunView.ts` 及测试仍包含 descriptor-missing legacy tree 分支。
- `runtime` / `server` / tests 仍维护 legacy source 与 downgrade 视图。

**Implication**  
只删 route 不够；protocol/frontend/runtime/tests 必须一起收口。

## Finding 7: `SubRunOutcome` 与 `AgentStatus` 当前同时表达 subrun 状态

- `crates/core/src/agent/mod.rs` 中同时存在 `AgentStatus` 与 `SubRunOutcome`。
- `SubRunResult.status` 使用 `SubRunOutcome`，而 `SubRunHandle.status`、`ChildAgentRef.status`、`ChildSessionNode.status` 使用 `AgentStatus`。
- `server/src/http/mapper.rs`、`runtime-execution` 和 `runtime-agent-tool` 里都有额外状态映射函数。

**Implication**  
这是同一业务语义的重复模型，应该收口为唯一状态词表。

## Finding 8: `SubRunDescriptor` 只是对已有 lineage 字段的重复壳

- `SubRunDescriptor` 只包含 `sub_run_id`、`parent_turn_id`、`parent_agent_id`、`depth`。
- `SubRunHandle` 已经持有这些字段中的绝大多数，但 `parent_turn_id` 仍是 `Option<String>`，并通过 `descriptor()` 额外产出 descriptor。
- `runtime` service contract 的 `SubRunStatusSnapshot` 继续暴露 `descriptor: Option<SubRunDescriptor>`。

**Implication**  
descriptor 和 optional `parent_turn_id` 正在为 downgrade 语义续命，而不是提供新的核心事实。

## Finding 9: execution receipt 在 `core` 与 `runtime` 中重复存在

- `core/src/runtime/traits.rs` 定义了 `PromptAccepted` 与 `RootExecutionAccepted`。
- `runtime/src/service/service_contract.rs` 又定义了 `PromptAccepted` 与 `AgentExecutionAccepted`。
- `runtime/src/service/execution/mod.rs` 在实现 trait 时继续做一次类型映射。

**Implication**  
这是一层没有业务价值的 receipt duplication，应收口为一个 canonical receipt。

## Finding 10: `launch_subagent` 当前挂在错误 owner 上

- `ExecutionOrchestrationBoundary` 负责 prompt submit / interrupt / root execute。
- `launch_subagent` 也挂在该 trait 上，但主要调用场景位于 `execution/context.rs` 的 deferred subagent executor，与 live child control 和 tool context 更相关。

**Implication**  
trait owner 不清晰，后续调用方更容易把 root orchestration 和 live child control 混在一起。

## Finding 11: `ChildAgentRef` 仍承载 UI 派生字段

- `ChildAgentRef` 在 core 中包含 `openable` 与 `open_session_id`。
- `ChildSessionNode::child_ref()` 在构造 child ref 时直接写死 `openable: true`。
- server / protocol / frontend tests 大量围绕该字段做序列化和 projection 断言。

**Implication**  
领域模型正在承担 view-only 便利字段，这会让 transport 与 UI 改动反过来污染 core。

## Finding 12: 并不是所有 summary 都是死代码

- `SubRunHandoff.summary` 仍是子 Agent 终态交接的重要摘要。
- `ChildSessionNotification.summary` 仍是父侧通知与 UI 消费的核心字段。

**Implication**  
真正可删的是无人消费的 summary projection，而不是全部 `summary` 字段。

## Finding 13: `action.rs` 存在隐式 contract，但当前代码没把“为什么”说清楚

- `ToolExecutionResult::model_content()` 实际会拼接错误信息与 child agent reference hint。
- `split_assistant_content()` 依赖 `to_ascii_lowercase()` 与原字符串共享相同字节偏移。

**Implication**  
这两处不补注释，后续很容易被“看起来能简化”的重构误伤。

## Finding 14: `ChildSessionNotification.open_session_id` 与 `child_ref.open_session_id` 重复

- `ChildSessionNotification` 外层持有 `open_session_id`。
- 同一通知里嵌套的 `child_ref` 已经包含 `open_session_id`。
- 两者当前表达同一个 child open target。

**Implication**  
同一事实在通知层被双写，后续任何 mapper、序列化或测试只要漏改一处，就会产生漂移风险。

## Finding 15: protocol 仍把 child/subrun 状态降成字符串

- `ChildAgentRefDto`、`ChildSessionNotificationDto` 等 DTO 把 `status` 定义为 `String`。
- core 层对应语义已经是正式枚举状态。

**Implication**  
protocol 在最该提供稳定边界的地方失去了类型安全，前端只能靠字符串匹配推断状态。

## Finding 16: compaction 原因的归一映射散在 runtime 调用点

- `HookCompactionReason` 允许 `Reactive`。
- durable `CompactTrigger` 只表达正式 trigger。
- 当前 `Reactive -> Auto` 的归一逻辑存在，但散在 runtime 实现里，而不是集中 owner。

**Implication**  
真正的问题不是“没有映射”，而是“映射没有单一事实源”，后续容易在新调用点继续复制手写逻辑。

## Finding 17: `PromptMetrics` 同一组字段在三层 event 里重复定义

- storage event 有一份完整字段集合。
- agent event 又定义一份同构字段集合。
- protocol event 再定义一份同构字段集合。

**Implication**  
这不是必要的 DTO 镜像成本，而是结构性重复；任何字段增删都需要三处同步搬运，极易失配。

- 当前导航由 `activeProjectId`、`activeSessionId`、`activeSubRunPath` 驱动。
- `App.tsx`、`sessionView.ts`、`store/reducer.ts` 与 focused subrun filter 构成真实导航主线。

**Implication**  
清理重点不是 router 框架代码，而是没有消费者的 API wrapper、projection、legacy 读模型分支和 duplicated open helper。

## Finding 2: `loadParentChildSummaryList`、`loadChildSessionView` 与 `buildParentSummaryProjection` 当前都没有 UI 消费者

- `frontend/src/lib/api/sessions.ts` 导出了 `loadParentChildSummaryList` 与 `loadChildSessionView`。
- `frontend/src/lib/subRunView.ts` 导出了 `buildParentSummaryProjection`、`ParentSummaryProjection`、`ChildSummaryCard`。
- 当前产品入口里没有调用这些导出；现有引用主要来自测试和文档。

**Implication**  
这组 surface 属于立即删除项，而不是“等以后再接”的预实现。

## Finding 3: `cancelSubRun` 仍是活跃主线，不属于立即删除项

- `SubRunBlock` 的取消按钮当前仍通过 `onCancelSubRun` 触发。
- `Chat`、`App`、`useAgent`、`frontend/src/lib/api/sessions.ts` 仍把动作接到 `/api/v1/sessions/{id}/subruns/{sub_run_id}/cancel`。

**Implication**  
必须先把 UI 切到 `closeAgent`，再删 legacy cancel route。

## Finding 4: 多个 server public surface 有实现、有测试，但没有当前产品入口

- `/api/v1/agents*`、`/api/v1/tools*`、`/api/runtime/plugins*`、`/api/config/reload` 都有 handler 或测试。
- 当前前端没有消费者，也没有明确 operator 面在产品里暴露这些能力。

**Implication**  
“有 handler / 有测试”不能成为保留理由。

## Finding 5: live 文档仍在宣传与主线不一致的能力面

- `docs/spec/agent-tool-and-api-spec.md` 仍把 `/api/v1/agents`、`/api/v1/tools` 等描述成当前 API 面。
- `docs/spec/session-and-subrun-spec.md` 仍在描述旧共享/独立子会话语义的演进阶段。
- `docs/spec/open-items.md` 还在把已决定删除的接口写成待讨论项。

**Implication**  
如果不更新 live 文档，仓库会继续暗示这些 surface 仍然受支持。

## Finding 6: `legacyDurable` 与 descriptorless downgrade 公开语义仍散落在前端、协议和运行时

- `frontend/src/types.ts` 仍公开 `legacyDurable` 和 related unsupported/legacy types。
- `subRunView.ts` 及测试仍包含 descriptor-missing legacy tree 分支。
- `runtime` / `server` / tests 仍维护 legacy source 与 downgrade 视图。

**Implication**  
只删 route 不够；protocol/frontend/runtime/tests 必须一起收口。

## Finding 7: `SubRunOutcome` 与 `AgentStatus` 当前同时表达 subrun 状态

- `crates/core/src/agent/mod.rs` 中同时存在 `AgentStatus` 与 `SubRunOutcome`。
- `SubRunResult.status` 使用 `SubRunOutcome`，而 `SubRunHandle.status`、`ChildAgentRef.status`、`ChildSessionNode.status` 使用 `AgentStatus`。
- `server/src/http/mapper.rs`、`runtime-execution` 和 `runtime-agent-tool` 里都有额外状态映射函数。

**Implication**  
这是同一业务语义的重复模型，应该收口为唯一状态词表。

## Finding 8: `SubRunDescriptor` 只是对已有 lineage 字段的重复壳

- `SubRunDescriptor` 只包含 `sub_run_id`、`parent_turn_id`、`parent_agent_id`、`depth`。
- `SubRunHandle` 已经持有这些字段中的绝大多数，但 `parent_turn_id` 仍是 `Option<String>`，并通过 `descriptor()` 额外产出 descriptor。
- `runtime` service contract 的 `SubRunStatusSnapshot` 继续暴露 `descriptor: Option<SubRunDescriptor>`。

**Implication**  
descriptor 和 optional `parent_turn_id` 正在为 downgrade 语义续命，而不是提供新的核心事实。

## Finding 9: execution receipt 在 `core` 与 `runtime` 中重复存在

- `core/src/runtime/traits.rs` 定义了 `PromptAccepted` 与 `RootExecutionAccepted`。
- `runtime/src/service/service_contract.rs` 又定义了 `PromptAccepted` 与 `AgentExecutionAccepted`。
- `runtime/src/service/execution/mod.rs` 在实现 trait 时继续做一次类型映射。

**Implication**  
这是一层没有业务价值的 receipt duplication，应收口为一个 canonical receipt。

## Finding 10: `launch_subagent` 当前挂在错误 owner 上

- `ExecutionOrchestrationBoundary` 负责 prompt submit / interrupt / root execute。
- `launch_subagent` 也挂在该 trait 上，但主要调用场景位于 `execution/context.rs` 的 deferred subagent executor，与 live child control 和 tool context 更相关。

**Implication**  
trait owner 不清晰，后续调用方更容易把 root orchestration 和 live child control 混在一起。

## Finding 11: `ChildAgentRef` 仍承载 UI 派生字段

- `ChildAgentRef` 在 core 中包含 `openable` 与 `open_session_id`。
- `ChildSessionNode::child_ref()` 在构造 child ref 时直接写死 `openable: true`。
- server / protocol / frontend tests 大量围绕该字段做序列化和 projection 断言。

**Implication**  
领域模型正在承担 view-only 便利字段，这会让 transport 与 UI 改动反过来污染 core。

## Finding 12: 并不是所有 summary 都是死代码

- `SubRunHandoff.summary` 仍是子 Agent 终态交接的重要摘要。
- `ChildSessionNotification.summary` 仍是父侧通知与 UI 消费的核心字段。

**Implication**  
真正可删的是无人消费的 summary projection，而不是全部 `summary` 字段。

## Finding 13: `action.rs` 存在隐式 contract，但当前代码没把“为什么”说清楚

- `ToolExecutionResult::model_content()` 实际会拼接错误信息与 child agent reference hint。
- `split_assistant_content()` 依赖 `to_ascii_lowercase()` 与原字符串共享相同字节偏移。

**Implication**  
这两处不补注释，后续很容易被“看起来能简化”的重构误伤。
