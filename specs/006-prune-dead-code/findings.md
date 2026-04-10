# Findings: 删除死代码与兼容层收口

本文件记录当前仓库里已经确认的事实，用来约束后续 plan 和实现，不再靠印象争论“到底是不是死代码”。

## Finding 1: 前端没有 React Router，所谓“前端路由死代码”主要是状态读模型与 projection

- 当前导航由 `activeProjectId`、`activeSessionId`、`activeSubRunPath` 驱动。
- `App.tsx`、`sessionView.ts`、`store/reducer.ts` 与 `buildFocusedSubRunFilter()` 构成了真实导航主线。
- 因此需要删除的不是 router 框架代码，而是没有当前消费者的辅助 projection、API wrapper 和 legacy 读模型分支。

**Implication**  
不能把仍在主线导航里使用的 `SubRunThreadTree`、`activeSubRunPath` 或 focused filter 误判成“路由死代码”。

## Finding 2: `loadParentChildSummaryList`、`loadChildSessionView` 与 `buildParentSummaryProjection` 当前都没有 UI 消费者

- `frontend/src/lib/api/sessions.ts` 导出了 `loadParentChildSummaryList` 与 `loadChildSessionView`。
- `frontend/src/lib/subRunView.ts` 导出了 `buildParentSummaryProjection`、`ParentSummaryProjection`、`ChildSummaryCard`。
- 当前产品入口中没有调用这些导出；现有引用主要来自测试、archive 文档和“以后可能用”的说明。

**Implication**  
这组 surface 属于“立即删除”候选，而不是“先迁移后删除”。

## Finding 3: `cancelSubRun` 仍是活跃主线，不属于立即删除项

- `SubRunBlock` 的“取消子会话”按钮当前仍通过 `onCancelSubRun` 触发。
- `Chat`、`App`、`useAgent`、`frontend/src/lib/api/sessions.ts` 一路把该动作接到 `/api/v1/sessions/{id}/subruns/{sub_run_id}/cancel`。
- 也就是说，虽然代码注释已经把它标成 legacy，但产品流程还在用它。

**Implication**  
必须先把 UI 切到 `closeAgent`，再删 legacy cancel route；不能把“标了 legacy”误当成“已经没人在用”。

## Finding 4: 多个 server public surface 有实现、有测试，但没有当前产品入口

- `/api/v1/agents` 与 `/api/v1/agents/{id}/execute` 有真实 handler 和测试，但当前前端没有消费者。
- `/api/v1/tools` 与 `/api/v1/tools/{id}/execute` 同样存在，其中 execute 还是明确的骨架返回。
- `/api/runtime/plugins`、`/api/runtime/plugins/reload`、`/api/config/reload` 有实现和测试，但当前产品没有入口暴露它们。

**Implication**  
这些 surface 不能再被“已经写了 handler”和“有测试”自动保活。

## Finding 5: 当前 live 文档仍在宣传与主线不一致的能力面

- `docs/spec/agent-tool-and-api-spec.md` 仍把 `SharedSession` 写成正式路径、把 `IndependentSession` 写成 experimental，并把 `/api/v1/agents`、`/api/v1/tools` 等描述成当前 API 面。
- `docs/spec/session-and-subrun-spec.md` 仍在描述 `SharedSession` 正式主线与 `IndependentSession` experimental 的架构状态。
- `docs/spec/open-items.md` 仍以“要不要保留 `/api/v1/tools/{id}/execute`”这类语气保留已经应该清理的历史尾巴。

**Implication**  
如果不更新 live 文档，仓库会继续暗示这些 surface 仍属于当前支持范围。

## Finding 6: legacy downgrade 公开语义仍散落在前端、协议和运行时

- `frontend/src/types.ts` 仍公开 `sharedSession`、`legacyDurable`、`unsupported_legacy_shared_history`、legacy-only notification kinds 等类型。
- `subRunView.ts` 及其测试仍包含 descriptor-missing legacy tree 的分支逻辑。
- `runtime-execution`、`server` 和相关测试仍在维护 legacy status source 与显式降级视图。

**Implication**  
只删 route 或删 projection 不够；如果要真正删兼容层，protocol/frontend/runtime/tests 必须一起收口。

## Finding 7: 不是所有 summary 都是死代码

- `SubRunHandoff.summary` 仍是子 Agent 终态交接的重要摘要。
- `ChildSessionNotification.summary` 仍是父侧通知和 UI 消费的核心字段。
- 真正可删的是“没有消费者的重复 summary projection”，而不是所有 summary 语义。

**Implication**  
清理时必须按“摘要事实”和“摘要投影”分层处理，不能做文本级的粗暴删除。
