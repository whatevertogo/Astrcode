# Findings: 删除死代码与冗余契约收口

本文件记录当前仓库里已经确认的事实，用来约束后续实现，不再靠印象判断"到底是不是冗余"。

## Finding 1: 前端没有 React Router，所谓"前端路由死代码"主要是状态读模型与 projection

- 当前导航由 `activeProjectId`、`activeSessionId`、`activeSubRunPath` 驱动。
- `App.tsx`、`sessionView.ts`、`store/reducer.ts` 与 focused subrun filter 构成真实导航主线。

**Implication**  
清理重点不是 router 框架代码，而是没有消费者的 API wrapper、projection、legacy 读模型分支和 duplicated open helper。

**Resolution (T012-T017)**：已删除无人消费的 projection 和 client wrapper，主线导航保留不变。

## Finding 2: `loadParentChildSummaryList`、`loadChildSessionView` 与 `buildParentSummaryProjection` 当前都没有 UI 消费者

- 这些 surface 无当前 UI 调用，只有测试/文档引用。

**Resolution (T012-T014)**：已直接删除 client + projection + server route + tests + docs。

## Finding 3: `cancelSubRun` 已切换到 `closeAgent` 主线

- 旧 cancel route 与 `cancelSubRun` client wrapper 已删除。
- UI 取消动作现在走 `closeAgent` 路径。

**Resolution (T027)**：UI cancel 切到 closeAgent，旧 cancel route 和 client wrapper 已删除。

## Finding 4: 多个 server public surface 有实现、有测试，但没有当前产品入口

- `/api/v1/agents*`、`/api/v1/tools*`、`/api/runtime/plugins*` 已在路由层保留（`/api/v1/agents` 仍有真实消费者）。
- `/api/runtime/plugins*` 的 skeleton handler 和测试已删除。

**Resolution (T015)**：skeleton routes 已删除；`/api/v1/agents` 和 `/api/v1/agents/{id}/execute` 保留为现役 surface。

## Finding 5: live 文档目录 `docs/spec/` 当前为空

- 原 `docs/spec/` 下的三个文档已在 Phase 1 阶段被清理。
- 当前 live spec 仅存在于 `specs/006-prune-dead-code/contracts/` 目录。

**Resolution (T030-T031)**：quickstart.md 已更新，live docs 目录无内容需修改。

## Finding 6: `legacyDurable` 保留为前端投影标识，descriptorless downgrade 已改为明确失败

- `legacyDurable` 在 `frontend/src/types.ts` 中保留，用于渲染稳定错误而非降级视图。
- descriptorless / legacyDurable 输入在 runtime/server 层已改为明确失败路径。

**Resolution (T022, T026)**：`legacyDurable` 保留为 `SubRunStatusSource` 的合法变体；旧 downgrade 视图已替换为结构化错误。

## Finding 7: `SubRunOutcome` 与 `AgentStatus` 重复问题已收口

- `SubRunOutcome` 已删除，所有 subrun 状态统一使用 `AgentStatus`。

**Resolution (T020)**：收口完成，唯一状态词表为 `AgentStatus`。

## Finding 8: `SubRunDescriptor` 已删除，lineage 字段直接在 `SubRunHandle` 上

- `SubRunDescriptor` 及其 `descriptor()` accessor 已删除。
- `parent_turn_id` 改为必填字段。

**Resolution (T020)**：收口完成，唯一 lineage owner 为 `SubRunHandle`。

## Finding 9: execution receipt 已统一到 `ExecutionAccepted`

- `PromptAccepted`、`RootExecutionAccepted`、runtime 重复 receipt 已全部删除。

**Resolution (T021)**：收口完成，唯一 internal receipt 为 `ExecutionAccepted`。

## Finding 10: `launch_subagent` 已迁移到正确 owner boundary

- `launch_subagent` 从 `ExecutionOrchestrationBoundary` 迁移到 `LiveSubRunControlBoundary`。

**Resolution (T023)**：trait owner 已清晰分离。

## Finding 11: `ChildAgentRef.openable` 已删除

- `openable` 字段已从 `ChildAgentRef` 中移除。
- 可打开性由 `open_session_id` 非空判断。

**Resolution (T022)**：core child ref 不再承载 UI 派生字段。

## Finding 12: 现役 summary 字段保留

- `SubRunHandoff.summary` 和 `ChildSessionNotification.summary` 仍为活跃主线字段。
- 无人消费的 `buildParentSummaryProjection` 已删除。

**Resolution (T012-T014)**：summary 投影已删除，summary 字段保留。

## Finding 13: `action.rs` 隐式 contract 注释

- `ToolExecutionResult::model_content()` 和 `split_assistant_content()` 的隐式 contract 应补充注释。

**Status**：非本次收口范围，保留为后续改进项。

## Finding 14: `ChildSessionNotification.open_session_id` 外层重复已删除

- notification 外层的 `open_session_id` 已删除。
- 唯一 canonical open target 为 `child_ref.open_session_id`。

**Resolution (T024)**：收口完成。

## Finding 15: protocol 状态已收口为强类型枚举

- `status: String` 已替换为 `AgentStatus` 强类型枚举。

**Resolution (T024)**：收口完成。

## Finding 16: compaction reason 映射已集中化

- `Reactive -> Auto` 的归一映射已集中到 core hook 层。

**Resolution (T025, T008)**：映射单一事实源已建立。

## Finding 17: `PromptMetrics` 三层重复已通过共享 payload 收口

- `PromptMetricsPayload` 作为唯一共享指标字段集合。
- storage/domain/protocol 三层通过引用该 payload 避免手工搬运。

**Resolution (T005)**：收口完成。
