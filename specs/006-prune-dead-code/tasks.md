# Tasks: 删除死代码与冗余契约收口

**Input**: Design documents from `D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/`
**Prerequisites**: [plan.md](D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/plan.md), [spec.md](D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/spec.md), [research.md](D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/research.md), [data-model.md](D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/data-model.md), [contracts/](D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/contracts/)

**Tests**: 必须包含。该特性删除 public surface、修改 durable/event/protocol 合同、调整 trait owner，并要求保留主线能力与明确失败边界。

**Organization**: 任务按用户故事分组，确保每个故事都可以独立实现和验证。

## Format: `[ID] [P?] [Story] Description`

- **[P]**: 可并行执行（不同文件、无未完成依赖）
- **[Story]**: 对应用户故事（`[US1]`、`[US2]`、`[US3]`）
- 每个任务都包含精确文件路径

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: 固化本次实现的事实输入、验证基线和迁移清单

- [ ] T001 刷新实现前 caller inventory 与保留 surface 基线到 `D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/findings.md`, `D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/migration.md`, `D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/contracts/retained-surface-contract.md`
- [ ] T002 [P] 固化本次 grep/手工回归/验证命令到 `D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/quickstart.md`
- [ ] T003 [P] 对齐本次 canonical contract 目标到 `D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/plan.md`, `D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/data-model.md`, `D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/design-subrun-contract-consolidation.md`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: 建立所有用户故事共享的 canonical contract、协议投影和验证基础

**⚠️ CRITICAL**: 该阶段完成前，不要开始任何用户故事实现

- [ ] T004 建立 canonical execution/status/lineage 合同骨架于 `D:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs`, `D:/GitObjectsOwn/Astrcode/crates/core/src/runtime/traits.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime/src/service/service_contract.rs`
- [ ] T005 [P] 引入共享 `PromptMetricsPayload` 与事件层复用入口于 `D:/GitObjectsOwn/Astrcode/crates/core/src/event/types.rs`, `D:/GitObjectsOwn/Astrcode/crates/core/src/event/domain.rs`, `D:/GitObjectsOwn/Astrcode/crates/core/src/event/translate.rs`
- [ ] T006 [P] 建立 canonical child open target 与重复字段删除策略于 `D:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime-execution/src/subrun.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime-session/src/session_state.rs`
- [ ] T007 [P] 建立 protocol 强类型状态 DTO 与 mapper 基础于 `D:/GitObjectsOwn/Astrcode/crates/protocol/src/http/agent.rs`, `D:/GitObjectsOwn/Astrcode/crates/protocol/src/http/event.rs`, `D:/GitObjectsOwn/Astrcode/crates/server/src/http/mapper.rs`
- [ ] T008 集中 compaction 原因到 durable trigger 的映射于 `D:/GitObjectsOwn/Astrcode/crates/core/src/hook.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime-agent-loop/src/compaction_runtime.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime-agent-loop/src/agent_loop.rs`
- [ ] T009 更新共享合同基线测试于 `D:/GitObjectsOwn/Astrcode/crates/protocol/tests/subrun_event_serialization.rs`, `D:/GitObjectsOwn/Astrcode/crates/server/src/tests/session_contract_tests.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime/src/service/execution/tests.rs`。关键断言：`AgentStatus::TokenExceeded` 序列化正确、`ExecutionAccepted` 字段完整、`PromptMetricsPayload` 三层结构一致

**Checkpoint**: canonical contract、协议投影和共享验证基线已建立，用户故事可开始并行推进

---

## Phase 3: User Story 1 - 收口正式支持面 (Priority: P1) 🎯 MVP

**Goal**: 删除当前产品没有消费的前端/服务端 surface，同时保留当前主线会话、child session 与摘要能力

**Independent Test**: 运行 orphan surface grep 检查，并手工验证当前会话浏览、消息提交、focused subrun 与 child session 直开仍然可用

### Tests for User Story 1

- [ ] T010 [P] [US1] 删除或改写 orphan frontend surface 覆盖于 `D:/GitObjectsOwn/Astrcode/frontend/src/lib/subRunView.test.ts`, `D:/GitObjectsOwn/Astrcode/frontend/src/lib/sessionHistory.test.ts`
- [ ] T011 [P] [US1] 删除 orphan route 合同断言并保留现役 surface 覆盖于 `D:/GitObjectsOwn/Astrcode/crates/server/src/tests/session_contract_tests.rs`

### Implementation for User Story 1

- [ ] T012 [P] [US1] 删除未消费的前端 session summary/view client 于 `D:/GitObjectsOwn/Astrcode/frontend/src/lib/api/sessions.ts`
- [ ] T013 [P] [US1] 删除未消费的 parent summary projection 与相关类型于 `D:/GitObjectsOwn/Astrcode/frontend/src/lib/subRunView.ts`, `D:/GitObjectsOwn/Astrcode/frontend/src/types.ts`
- [ ] T014 [US1] 移除 child summary/view HTTP route 与 DTO 导出于 `D:/GitObjectsOwn/Astrcode/crates/server/src/http/routes/sessions/query.rs`, `D:/GitObjectsOwn/Astrcode/crates/server/src/http/routes/mod.rs`, `D:/GitObjectsOwn/Astrcode/crates/protocol/src/http/session.rs`, `D:/GitObjectsOwn/Astrcode/crates/protocol/src/http/mod.rs`
- [ ] T015 [US1] 删除无人消费的 v1/runtime skeleton route 于 `D:/GitObjectsOwn/Astrcode/crates/server/src/http/routes/runtime.rs`, `D:/GitObjectsOwn/Astrcode/crates/server/src/http/routes/tools.rs`, `D:/GitObjectsOwn/Astrcode/crates/server/src/http/routes/mod.rs`, `D:/GitObjectsOwn/Astrcode/crates/protocol/src/http/mod.rs`
- [ ] T016 [US1] 清理 retained frontend flow 对已删除 surface 的调用于 `D:/GitObjectsOwn/Astrcode/frontend/src/App.tsx`, `D:/GitObjectsOwn/Astrcode/frontend/src/hooks/useAgent.ts`, `D:/GitObjectsOwn/Astrcode/frontend/src/lib/api/sessions.ts`
- [ ] T017 [US1] 保持现役 summary 与 child navigation 主线可用并移除重复 open flag 依赖于 `D:/GitObjectsOwn/Astrcode/frontend/src/lib/agentEvent.ts`, `D:/GitObjectsOwn/Astrcode/frontend/src/lib/subRunView.ts`, `D:/GitObjectsOwn/Astrcode/crates/server/src/http/routes/sessions/query.rs`

**Checkpoint**: User Story 1 完成后，正式支持面已明显收口，且主线浏览/提交/child session 直开保持可用

---

## Phase 4: User Story 2 - 删除旧模型兼容层 (Priority: P1)

**Goal**: 删除 legacy/duplicate subrun contract，收口到唯一状态、唯一 receipt、唯一 open target、唯一 compaction 映射与明确失败边界

**Independent Test**: 运行 canonical contract grep 与协议/事件序列化测试，确认 `SubRunOutcome`、`SubRunDescriptor`、外层 `open_session_id`、`status: String` 和三层重复 metrics payload 全部退出主线

### Tests for User Story 2

- [ ] T018 [P] [US2] 更新 core/runtime canonical contract 回归测试于 `D:/GitObjectsOwn/Astrcode/crates/core/src/event/types.rs`, `D:/GitObjectsOwn/Astrcode/crates/core/src/event/translate.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime/src/service/execution/tests.rs`。关键断言：`SubRunOutcome` 变体已映射到 `AgentStatus`、`SubRunDescriptor` 已删除、`parent_turn_id` 必填
- [ ] T019 [P] [US2] 更新 child ref、protocol DTO 和 compaction 相关序列化测试于 `D:/GitObjectsOwn/Astrcode/crates/protocol/tests/subrun_event_serialization.rs`, `D:/GitObjectsOwn/Astrcode/crates/server/src/tests/session_contract_tests.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime-agent-loop/src/agent_loop/tests/hook.rs`。关键断言：`ChildAgentRef` 不含 `openable`、protocol 状态为强类型枚举非字符串、notification 无外层 `open_session_id`、compaction 映射只有一条路径

### Implementation for User Story 2

- [ ] T020 [P] [US2] 将 `SubRunOutcome`、`TokenExceeded` 与必填 `parent_turn_id` 收口到 canonical core 类型，并删除 `SubRunDescriptor` 及其 `descriptor()` accessor 于 `D:/GitObjectsOwn/Astrcode/crates/core/src/agent/mod.rs`, `D:/GitObjectsOwn/Astrcode/crates/core/src/runtime/traits.rs`
- [ ] T021 [P] [US2] 删除重复 execution receipt 并统一 `ExecutionAccepted` 于 `D:/GitObjectsOwn/Astrcode/crates/core/src/runtime/traits.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime/src/service/service_contract.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime/src/service/execution/mod.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime/src/service/execution/root.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime/src/service/turn/submit.rs`
- [ ] T022 [US2] 让 durable/session 写入与 child notification 使用 canonical child ref、共享 metrics payload，并删除 `ChildAgentRef.openable` 字段 于 `D:/GitObjectsOwn/Astrcode/crates/runtime-execution/src/subrun.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime-session/src/session_state.rs`, `D:/GitObjectsOwn/Astrcode/crates/core/src/event/types.rs`, `D:/GitObjectsOwn/Astrcode/crates/core/src/event/domain.rs`, `D:/GitObjectsOwn/Astrcode/crates/core/src/event/translate.rs`
- [ ] T023 [US2] 把 subagent launch 与 live control 收口到正确 owner boundary 于 `D:/GitObjectsOwn/Astrcode/crates/core/src/runtime/traits.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime-agent-control/src/lib.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime/src/service/execution/context.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime/src/service/execution/subagent.rs`
- [ ] T024 [US2] 用强类型状态 DTO 替换 protocol `status: String` 并移除外层 `open_session_id` 于 `D:/GitObjectsOwn/Astrcode/crates/protocol/src/http/agent.rs`, `D:/GitObjectsOwn/Astrcode/crates/protocol/src/http/event.rs`, `D:/GitObjectsOwn/Astrcode/crates/protocol/src/http/session.rs`, `D:/GitObjectsOwn/Astrcode/crates/server/src/http/mapper.rs`, `D:/GitObjectsOwn/Astrcode/crates/server/src/http/routes/sessions/query.rs`
- [ ] T025 [US2] 集中 compaction reason 映射并保留 `Reactive` 的内部语义于 `D:/GitObjectsOwn/Astrcode/crates/core/src/hook.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime-agent-loop/src/compaction_runtime.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime-agent-loop/src/agent_loop.rs`
- [ ] T026 [US2] 为 descriptorless、legacyDurable 与旧共享历史输入构建明确失败路径：runtime/server 层拒绝旧语义并返回结构化错误（含缺失字段与不支持的旧语义标识），不再返回 downgrade 视图或"部分可用"结果 于 `D:/GitObjectsOwn/Astrcode/crates/runtime-execution/src/subrun.rs`, `D:/GitObjectsOwn/Astrcode/crates/server/src/http/routes/sessions/query.rs`, `D:/GitObjectsOwn/Astrcode/crates/server/src/http/mapper.rs`
- [ ] T027 [US2] 将前端 cancel 按钮从旧 cancel route 切换到 `closeAgent`，删除旧 cancel route 与 client wrapper 于 `D:/GitObjectsOwn/Astrcode/frontend/src/hooks/useAgent.ts`, `D:/GitObjectsOwn/Astrcode/frontend/src/lib/api/sessions.ts`, `D:/GitObjectsOwn/Astrcode/crates/server/src/http/routes/runtime.rs`, `D:/GitObjectsOwn/Astrcode/crates/protocol/src/http/agent.rs`
- [ ] T028 [US2] 更新 frontend subrun/session read model 以消费强类型状态与嵌套 open target 于 `D:/GitObjectsOwn/Astrcode/frontend/src/types.ts`, `D:/GitObjectsOwn/Astrcode/frontend/src/lib/agentEvent.ts`, `D:/GitObjectsOwn/Astrcode/frontend/src/lib/subRunView.ts`, `D:/GitObjectsOwn/Astrcode/frontend/src/hooks/useAgent.ts`（注意：与 US1 共享 `useAgent.ts`，需在 US1 完成后串行执行）

**Checkpoint**: User Story 2 完成后，legacy/duplicate subrun contract 已收口为唯一正式表达，旧 downgrade 输入只剩明确失败

---

## Phase 5: User Story 3 - 文档、测试与术语同步收口 (Priority: P2)

**Goal**: 让 live 文档、测试和夹具只描述保留 surface 与明确失败边界

**Independent Test**: 运行文档 grep 检查并确认 live spec 不再宣传已删除 surface；测试夹具不再维持 legacy route、legacy status 或外层重复字段

### Tests for User Story 3

- [ ] T029 [P] [US3] 删除或改写已删除 surface 的前端与运行时夹具测试于 `D:/GitObjectsOwn/Astrcode/frontend/src/lib/subRunView.test.ts`, `D:/GitObjectsOwn/Astrcode/crates/runtime-agent-loop/src/agent_loop/tests/fixtures.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime/src/service/execution/tests.rs`
- [ ] T030 [P] [US3] 刷新文档驱动的 grep/回归检查于 `D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/quickstart.md`, `D:/GitObjectsOwn/Astrcode/docs/spec/agent-tool-and-api-spec.md`, `D:/GitObjectsOwn/Astrcode/docs/spec/session-and-subrun-spec.md`, `D:/GitObjectsOwn/Astrcode/docs/spec/open-items.md`

### Implementation for User Story 3

- [ ] T031 [P] [US3] 更新 live docs 以移除已删除 surface 并记录保留合同于 `D:/GitObjectsOwn/Astrcode/docs/spec/agent-tool-and-api-spec.md`, `D:/GitObjectsOwn/Astrcode/docs/spec/session-and-subrun-spec.md`, `D:/GitObjectsOwn/Astrcode/docs/spec/open-items.md`
- [ ] T032 [US3] 为 `contracts/retained-surface-contract.md` 中每个保留 surface 补充用途、消费者类型和所属边界说明（FR-014）于 `D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/contracts/retained-surface-contract.md`
- [ ] T033 [US3] 同步 feature 三层文档与最终实现口径于 `D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/findings.md`, `D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/migration.md`, `D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/quickstart.md`, `D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/contracts/retained-surface-contract.md`, `D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/contracts/summary-and-navigation-contract.md`, `D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/contracts/legacy-failure-and-control-cutover.md`, `D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/contracts/subrun-canonical-contract.md`

**Checkpoint**: User Story 3 完成后，文档、夹具和测试只描述保留主线与明确失败边界

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: 收尾多故事交叉影响，完成仓库级验证

- [ ] T034 [P] 审计修改过的 runtime 路径的日志级别、错误上下文、锁恢复与异步句柄管理于 `D:/GitObjectsOwn/Astrcode/crates/runtime/src/service/execution/mod.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime/src/service/execution/status.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime/src/service/session/mod.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime-agent-control/src/lib.rs`, `D:/GitObjectsOwn/Astrcode/crates/runtime-agent-loop/src/agent_loop.rs`
- [ ] T035 运行仓库级验证命令并记录结果于 `D:/GitObjectsOwn/Astrcode/specs/006-prune-dead-code/quickstart.md`。必须逐项验证以下 6 类主线回归场景：(1) 当前会话浏览——SSE 历史回放与增量订阅一致；(2) 当前子执行聚焦——focused subrun 状态与摘要正确展示；(3) 当前子会话直开——child session 通过 canonical open target 打开；(4) 当前配置读写——模型枚举与连通性测试可用；(5) 当前消息提交——submit prompt 完整流程通过；(6) 当前活跃子执行控制——closeAgent 取消/关闭子 agent 成功

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)**: 无依赖，可立即开始
- **Phase 2 (Foundational)**: 依赖 Phase 1 完成；阻塞全部用户故事
- **Phase 3 (US1)**: 依赖 Phase 2 完成
- **Phase 4 (US2)**: 依赖 Phase 2 完成；可与 US1 并行，但共享 `frontend/src/hooks/useAgent.ts` 与 `crates/server/src/http/routes/sessions/query.rs` 时需串行合并
- **Phase 5 (US3)**: 依赖 US1 与 US2 完成后的最终实现口径
- **Phase 6 (Polish)**: 依赖所有目标用户故事完成

### User Story Dependencies

- **US1 (P1)**: 完成后即可作为 MVP 展示当前正式支持面的收口效果
- **US2 (P1)**: 依赖 Foundational；建议在 US1 核心删除完成后并入，以减少共享文件冲突
- **US3 (P2)**: 依赖 US1 + US2，确保文档和测试反映最终合同

### Within Each User Story

- 先更新该故事的测试/合同覆盖，再完成实现
- 先改 core/protocol canonical contract，再改 server/frontend 投影
- 先迁移活跃入口，再删除 legacy surface
- 每个故事完成后，都要能独立执行其 Independent Test

### Parallel Opportunities

- Setup 阶段的 `T002`、`T003` 可并行
- Foundational 阶段的 `T005`、`T006`、`T007` 可并行
- US1 的 `T010`、`T011`、`T012`、`T013` 可并行
- US2 的 `T018`、`T019`、`T020`、`T021` 可并行；`T028` 需在 US1 完成后执行
- US3 的 `T029`、`T030`、`T031` 可并行

---

## Parallel Example: User Story 1

```bash
# 并行准备 US1 的测试与前端删除项
Task: "删除或改写 orphan frontend surface 覆盖于 frontend/src/lib/subRunView.test.ts, frontend/src/lib/sessionHistory.test.ts"
Task: "删除 orphan route 合同断言并保留现役 surface 覆盖于 crates/server/src/tests/session_contract_tests.rs"
Task: "删除未消费的前端 session summary/view client 于 frontend/src/lib/api/sessions.ts"
Task: "删除未消费的 parent summary projection 与相关类型于 frontend/src/lib/subRunView.ts, frontend/src/types.ts"
```

---

## Parallel Example: User Story 2

```bash
# 并行推进 core/protocol 基础收口
Task: "更新 core/runtime canonical contract 回归测试于 crates/core/src/event/types.rs, crates/core/src/event/translate.rs, crates/runtime/src/service/execution/tests.rs"
Task: "将 SubRunOutcome、TokenExceeded 与必填 parent_turn_id 收口到 canonical core 类型于 crates/core/src/agent/mod.rs, crates/core/src/runtime/traits.rs"
Task: "删除重复 execution receipt 并统一 ExecutionAccepted 于 crates/core/src/runtime/traits.rs, crates/runtime/src/service/service_contract.rs, crates/runtime/src/service/execution/mod.rs, crates/runtime/src/service/execution/root.rs, crates/runtime/src/service/turn/submit.rs"
Task: "集中 compaction reason 映射并保留 Reactive 的内部语义于 crates/core/src/hook.rs, crates/runtime-agent-loop/src/compaction_runtime.rs, crates/runtime-agent-loop/src/agent_loop.rs"
```

---

## Parallel Example: User Story 3

```bash
# 并行刷新文档与夹具
Task: "删除或改写已删除 surface 的前端与运行时夹具测试于 frontend/src/lib/subRunView.test.ts, crates/runtime-agent-loop/src/agent_loop/tests/fixtures.rs, crates/runtime/src/service/execution/tests.rs"
Task: "刷新文档驱动的 grep/回归检查于 specs/006-prune-dead-code/quickstart.md, docs/spec/agent-tool-and-api-spec.md, docs/spec/session-and-subrun-spec.md, docs/spec/open-items.md"
Task: "更新 live docs 以移除已删除 surface 并记录保留合同于 docs/spec/agent-tool-and-api-spec.md, docs/spec/session-and-subrun-spec.md, docs/spec/open-items.md"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. 完成 Phase 1: Setup
2. 完成 Phase 2: Foundational
3. 完成 Phase 3: US1
4. 运行 US1 的独立验证
5. 先展示“正式支持面已收口、主线功能未损坏”的结果

### Incremental Delivery

1. Setup + Foundational 完成后，建立 canonical contract 与共享验证基线
2. 交付 US1，先收口 orphan surface
3. 交付 US2，收口 duplicate/legacy subrun contract
4. 交付 US3，收尾文档、夹具和 live spec
5. 最后执行 Polish 阶段完成仓库级验证

### Parallel Team Strategy

1. 一人推进 Foundational 的 core/runtime canonical contract
2. 一人准备 US1 的 frontend/server orphan surface 删除
3. 一人准备 US2 的 protocol/server/frontend 状态 DTO 与 open target 改造
4. US1 + US2 合并后，再由一人统一完成 US3 文档/测试收口与 Polish 验证

---

## Notes

- `[P]` 任务只适用于不同文件、无未完成依赖的场景
- `US1` 是建议 MVP 范围
- 任何删除 public surface 的任务都必须先确认调用方已迁移或不存在
- 任何触及 durable event、protocol DTO、trait owner 的任务都必须保留对应验证
