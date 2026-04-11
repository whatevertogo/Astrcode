# Migration: 删除死代码与冗余契约收口

## Migration Principles

- 先做 caller inventory，再删公共入口。
- 先建立 canonical owner，再删重复模型。
- 无消费者 surface 立即删除，不做观察期。
- 活跃主线先迁移，再删 legacy 入口。
- live 文档、测试、夹具与实现必须同批次收口。

## Caller Inventory — Final State

| Surface | Action Taken | Task |
|--------|--------------|------|
| `loadParentChildSummaryList` | 已删除 client + tests | T012 |
| `loadChildSessionView` | 已删除 client + tests | T012 |
| `buildParentSummaryProjection` | 已删除 projection + types + tests | T013 |
| `/api/sessions/{id}/children/summary` | 已删除 route + DTO + tests | T014 |
| `/api/sessions/{id}/children/{child_session_id}/view` | 已删除 route + DTO + tests | T014 |
| `/api/v1/tools*` | 已删除 skeleton route + tests | T015 |
| `/api/runtime/plugins*` | 已删除 skeleton route + tests | T015 |
| `/api/v1/agents` | **保留**（有前端 Agent 选择 UI 消费者） | — |
| `/api/v1/agents/{id}/execute` | **保留**（有前端 root execution 入口消费者） | — |
| `/api/config/reload` | **保留**（CLI / 开发工具消费者） | — |
| `SubRunOutcome` | 已收口到 `AgentStatus` | T020 |
| `SubRunDescriptor` | 已删除，lineage 直接在 `SubRunHandle` | T020 |
| `parent_turn_id` | 已改为必填 | T020 |
| `PromptAccepted` / `RootExecutionAccepted` / runtime duplicates | 已收口到 `ExecutionAccepted` | T021 |
| `launch_subagent` | 已迁移到 `LiveSubRunControlBoundary` | T023 |
| `ChildAgentRef.openable` | 已删除 | T022 |
| `ChildSessionNotification.open_session_id` 外层 | 已删除，只保留 `child_ref.open_session_id` | T024 |
| child/subrun DTO `status: String` | 已收口到强类型 `AgentStatus` 枚举 | T024 |
| 三层 `PromptMetrics` variant 字段重复 | 已提取共享 `PromptMetricsPayload` | T005 |
| 散落的 `Reactive -> CompactTrigger` 手写映射 | 已集中到 core hook 层 | T008, T025 |
| `cancelSubRun` client + cancel route | UI 已切到 `closeAgent`，旧 route 已删除 | T027 |
| `legacyDurable` downgrade | 保留为前端投影标识，runtime/server 层改为明确失败 | T026 |

## Phase Order — Completion Status

### Phase 1: Setup (T001-T003) ✅

锁定支持面与 canonical owner，建立 caller inventory、grep 命令和 canonical contract 目标。

### Phase 2: Foundational (T004-T009) ✅

建立 canonical contract 骨架、共享 payload、protocol 强类型 DTO 和共享验证基线。

### Phase 3: US1 — 收口正式支持面 (T010-T017) ✅

删除 orphan frontend surface、server route、skeleton route。主线浏览/提交/child session 直开保留可用。

### Phase 4: US2 — 删除旧模型兼容层 (T018-T028) ✅

收口 `SubRunOutcome` → `AgentStatus`、`SubRunDescriptor` → `SubRunHandle` 必填字段、重复 receipt → `ExecutionAccepted`、`openable` 删除、强类型状态枚举、compaction 映射集中化、明确失败路径、closeAgent 切换、前端 read model 更新。

### Phase 5: US3 — 文档、测试与术语同步收口 (T029-T033) ✅

清理重复测试夹具、dead code fixture、更新 quickstart.md grep 检查、更新 retained-surface-contract.md、同步 findings.md 和 migration.md。

### Phase 6: Polish & Cross-Cutting (T034-T035)

审计日志/错误/锁管理，运行仓库级验证。

## Validation Matrix

见 `quickstart.md` 的静态检查与自动化验证章节。

## Rollback Considerations

- 不建议回滚到"双轨状态模型 + descriptor downgrade + legacy cancel route"的状态，因为那会重新引入支持面歧义。
- 如果某条被删除的 surface 真有遗漏消费者，应先补 caller inventory 与 owner 说明，再重新设计，而不是直接恢复旧壳。
