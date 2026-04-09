---
description: "Task list for 子智能体会话与缓存边界优化"
---

# Tasks: 子智能体会话与缓存边界优化

**Input**: Design documents from `/specs/004-subagent-cache-context/`  
**Prerequisites**: `plan.md`, `spec.md`, `research.md`, `data-model.md`, `contracts/`, `quickstart.md`, `migration.md`

**Tests**: 本特性会修改 durable event、resume/replay、prompt cache、父子唤醒链路、protocol DTO 与 `/history`/`/events` 投影，因此测试任务为强制项，不允许省略。

**Organization**: Tasks are grouped by user story so each story can be implemented, tested, and validated independently.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel after prerequisite dependencies are satisfied
- **[Story]**: Which user story this task belongs to (`US1`-`US5`)
- Every task includes exact file paths

## Path Conventions

- Rust backend crates live under `crates/`
- HTTP/SSE DTOs and transport contracts live under `crates/protocol/` and `crates/server/`
- Frontend state, API clients, and session projections live under `frontend/src/`
- Feature docs and validation commands live under `specs/004-subagent-cache-context/`

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: 锁定 cutover 范围、测试夹具和验收基线，避免后续实现继续沿用旧兼容前提

- [x] T001 Capture caller inventory, cutover checkpoints, and public-surface removal notes in `specs/004-subagent-cache-context/findings.md` and `specs/004-subagent-cache-context/migration.md`
- [x] T002 [P] Create shared backend fixtures for child-session spawn, replay-based resume, delivery buffering, and legacy rejection in `crates/runtime/src/service/execution/tests.rs`, `crates/runtime-agent-loop/src/agent_loop/tests/fixtures.rs`, and `crates/server/src/tests/test_support.rs`
- [x] T003 [P] Create frontend/session fixtures for parent summary projection, child session entry links, and legacy error display in `frontend/src/lib/sessionHistory.test.ts`, `frontend/src/lib/sessionView.test.ts`, and `frontend/src/lib/agentEvent.test.ts`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: 建立所有故事共享的 durable 契约、协议字段、投影入口与可观测基础

**⚠️ CRITICAL**: No user story work should start until this phase is complete

- [x] T004 Define canonical `ChildSessionNode`, `SubRun`, boundary-fact, and legacy-rejection fields in `crates/core/src/agent/mod.rs`, `crates/core/src/event/domain.rs`, `crates/core/src/event/types.rs`, `crates/protocol/src/http/agent.rs`, and `crates/protocol/src/http/event.rs`
- [x] T005 [P] Establish shared parent projection and legacy rejection plumbing in `crates/core/src/projection/agent_state.rs`, `crates/server/src/http/routes/sessions/filter.rs`, and `crates/server/src/http/routes/sessions/query.rs`
- [x] T006 [P] Wire structured observability and diagnostics for child lifecycle, lineage mismatch, cache reuse, delivery buffering, and legacy rejection in `crates/runtime/src/service/observability.rs`, `crates/runtime-execution/src/lib.rs`, and `crates/runtime-prompt/src/diagnostics.rs`
- [x] T007 [P] Align transport/store vocabulary for `childSessionId`, `executionId`, status sources, and unsupported legacy errors in `frontend/src/types.ts`, `frontend/src/lib/api/sessions.ts`, and `frontend/src/lib/agentEvent.ts`
- [x] T008 Add foundational round-trip coverage for boundary facts, event translation, and legacy rejection in `crates/core/src/event/translate.rs`, `crates/protocol/tests/subrun_event_serialization.rs`, and `crates/server/src/tests/session_contract_tests.rs`

**Checkpoint**: Foundation ready. Shared durable contracts, projection entry points, and diagnostics are stable enough for story work.

---

## Phase 3: User Story 1 - 子智能体拥有独立会话真相 (Priority: P1) 🎯 MVP

**Goal**: 让每个新子智能体从创建开始就拥有独立 child session durable 历史，父历史只保留边界事实与稳定入口。

**Independent Test**: 在同一父会话下连续创建多个子智能体，验证每个子智能体拥有独立会话身份和独立历史，且父会话历史只保留边界事件与结果摘要。

### Tests for User Story 1

- [x] T009 [P] [US1] Add runtime regression tests for independent child-session spawn, parent-history cleanliness, and legacy shared-history rejection in `crates/runtime/src/service/execution/tests.rs` and `crates/runtime-execution/src/subrun.rs`
- [x] T010 [P] [US1] Add server/frontend projection tests proving parent history exposes only summary facts and child entry links in `crates/server/src/tests/session_contract_tests.rs`, `frontend/src/lib/sessionHistory.test.ts`, and `frontend/src/lib/subRunView.test.ts`

### Implementation for User Story 1

- [x] T011 [US1] Remove the independent-session experimental gate and any new-write fallback to shared history in `crates/runtime-execution/src/policy.rs`, `crates/core/src/agent/mod.rs`, and `crates/runtime/src/service/execution/subagent.rs`
- [x] T012 [US1] Persist `ChildSessionNode` and initial child-session durable records during spawn in `crates/runtime/src/service/execution/subagent.rs`, `crates/runtime-session/src/session_state.rs`, and `crates/runtime-session/src/turn_runtime.rs`
- [x] T013 [US1] Delete shared-history read/replay/recovery paths and return `unsupported_legacy_shared_history` for legacy data in `crates/runtime-session/src/lib.rs`, `crates/runtime-session/src/session_state.rs`, and `crates/server/src/http/routes/sessions/query.rs`
- [x] T014 [US1] Restrict parent projections to child boundary facts and stable open-session links in `crates/core/src/projection/agent_state.rs`, `crates/core/src/event/translate.rs`, and `crates/server/src/http/routes/sessions/filter.rs`
- [x] T015 [US1] Shift the default parent read model toward summary cards and child-session entry links in `frontend/src/lib/subRunView.ts`, `frontend/src/components/Chat/SubRunBlock.tsx`, and `frontend/src/App.tsx`
- [x] T016 [US1] Add spawn, child-session creation, and legacy rejection error context to logs in `crates/runtime-execution/src/lib.rs`, `crates/runtime/src/service/observability.rs`, and `crates/runtime-session/src/support.rs`

**Checkpoint**: 新子智能体默认进入独立 child session durable 模型，父历史不再混入子内部事件，legacy 共享历史被显式拒绝。

---

## Phase 4: User Story 2 - 恢复沿用原子会话而不是重开新会话 (Priority: P1)

**Goal**: 让 resume 真正基于 child session durable replay 继续原会话，而不是伪装成新的并列 spawn。

**Independent Test**: 让子智能体中途停止后再执行恢复，验证恢复前后子会话身份不变、子执行实例变化、恢复后继续基于原历史工作。

### Tests for User Story 2

- [x] T017 [P] [US2] Add regression tests for replay-based resume keeping `child_session_id` stable and minting new `SubRun` ids in `crates/runtime/src/service/execution/tests.rs` and `crates/runtime-agent-loop/src/agent_loop/tests/regression.rs`
- [x] T018 [P] [US2] Add failure-path tests for lineage mismatch, damaged child history, and unsafe resume rejection in `crates/runtime/src/service/execution/tests.rs` and `crates/server/src/tests/session_contract_tests.rs`

### Implementation for User Story 2

- [x] T019 [US2] Replace empty-state child resume assembly with durable replay or projector restoration in `crates/runtime-execution/src/prep.rs`, `crates/runtime/src/service/execution/collaboration.rs`, and `crates/runtime/src/service/execution/subagent.rs`
- [x] T020 [US2] Persist resumed `SubRun` facts and parent-visible `child_resumed` boundary events in `crates/runtime-execution/src/subrun.rs`, `crates/runtime/src/service/execution/status.rs`, and `crates/core/src/event/domain.rs`
- [x] T021 [US2] Detect lineage conflicts and emit parent-visible plus diagnostic errors before aborting resume in `crates/runtime-execution/src/context.rs`, `crates/runtime/src/service/execution/collaboration.rs`, and `crates/runtime/src/service/observability.rs`
- [x] T022 [US2] Keep resume projections and child-session reopen routes stable across reloads in `crates/server/src/http/routes/sessions/query.rs`, `frontend/src/lib/api/sessions.ts`, and `frontend/src/lib/sessionView.ts`

**Checkpoint**: resume 成功时始终沿用原 `child_session_id`，失败时明确暴露错误且绝不静默降级为新 spawn。

---

## Phase 5: User Story 3 - 父背景以结构化方式继承给子智能体 (Priority: P2)

**Goal**: 让父传子的背景通过 prompt system blocks 结构化继承，首条任务消息只表达任务目标。

**Independent Test**: 在同一父会话下启动多个相似子智能体，验证子智能体首条任务消息只描述任务本身，父背景通过独立继承块传递。

### Tests for User Story 3

- [x] T023 [P] [US3] Add prompt/runtime tests proving child task messages stay task-only while inherited blocks render in `LayeredPromptBuilder.Inherited` between `SemiStable` and `Dynamic` in `crates/runtime-agent-loop/src/agent_loop/tests/prompt.rs` and `crates/runtime/src/service/execution/tests.rs`
- [x] T024 [P] [US3] Add regression tests for deterministic recent-tail clipping and zero durable `UserMessage` leakage from inherited context in `crates/runtime-agent-loop/src/context_window/compaction.rs` and `crates/core/src/event/translate.rs`

### Implementation for User Story 3

- [x] T025 [US3] Split `resolve_context_snapshot()` into `task_payload` and inherited context blocks in `crates/runtime-execution/src/context.rs` and `crates/runtime-execution/src/prep.rs`
- [x] T026 [US3] Extend prompt declaration plumbing so inherited blocks render in `LayeredPromptBuilder` 的 `Inherited` 层（位于 `SemiStable` 之后、`Dynamic` 之前）且不进入消息流 in `crates/runtime-prompt/src/prompt_declaration.rs`, `crates/runtime-prompt/src/context.rs`, `crates/runtime-prompt/src/layered_builder.rs`, and `crates/runtime/src/service/loop_factory.rs`
- [x] T027 [US3] Implement deterministic recent-tail filtering, tool-output summarization, and budget clipping in `crates/runtime-agent-loop/src/context_pipeline.rs`, `crates/runtime-agent-loop/src/context_window/compaction.rs`, and `crates/runtime-agent-loop/src/request_assembler.rs`
- [x] T028 [US3] Update child request assembly so only task payload enters message history and inherited context stays in prompt metadata in `crates/runtime-agent-loop/src/request_assembler.rs`, `crates/runtime/src/service/execution/context.rs`, and `crates/runtime/src/runtime_surface_assembler.rs`

**Checkpoint**: child 首条任务消息与父背景彻底分层，recent tail 经过确定性治理后进入 system prompt。

---

## Phase 6: User Story 4 - 相似子智能体安全复用稳定缓存 (Priority: P2)

**Goal**: 让父子智能体在强指纹一致时复用稳定 prompt 缓存，并在输入变化时安全失效。

**Independent Test**: 在相同工作条件下重复启动相似子智能体，验证后续启动的稳定上下文准备成本显著下降，且关键上下文维度变化时不会误命中旧缓存。

### Tests for User Story 4

- [x] T029 [P] [US4] Add prompt-cache regression tests for strong-fingerprint hit and miss behavior across matching and changed child inputs in `crates/runtime-prompt/src/layered_builder.rs` and `crates/runtime/src/service/turn/tests.rs`
- [x] T030 [P] [US4] Add telemetry tests for provider cache-metric support and `cache_creation_input_tokens` translation in `crates/runtime-llm/src/anthropic.rs`, `crates/runtime-llm/src/openai.rs`, and `crates/core/src/event/translate.rs`

### Implementation for User Story 4

- [x] T031 [US4] Inject shared `LayerCache` instances into child prompt builders and give the `Inherited` layer its own reusable cache segment in `crates/runtime-prompt/src/layered_builder.rs`, `crates/runtime/src/service/mod.rs`, and `crates/runtime/src/service/loop_factory.rs`
- [x] T032 [US4] Route cache reuse decisions through `runtime-prompt` fingerprint inputs and invalidation reasons so `compact_summary` and `recent_tail` each form an independent inherited-layer cache boundary in `crates/runtime-prompt/src/context.rs`, `crates/runtime-prompt/src/contributors/agents_md.rs`, and `crates/runtime-execution/src/context.rs`
- [x] T033 [US4] Surface cache hit or miss diagnostics and supported-provider gating for `SC-003` in `crates/runtime-prompt/src/diagnostics.rs`, `crates/runtime-llm/src/lib.rs`, `crates/runtime-llm/src/openai.rs`, and `crates/runtime-llm/src/anthropic.rs`
- [x] T034 [US4] Emit cache reuse metadata through execution observability and prompt metrics events in `crates/runtime/src/service/observability.rs`, `crates/runtime-agent-loop/src/agent_loop/turn_runner.rs`, and `crates/core/src/event/types.rs`

**Checkpoint**: 相似 child 的稳定上下文可安全复用，缓存命中/失效与 provider 指标都可直接观察。

---

## Phase 7: User Story 5 - 子交付可靠唤醒父智能体且不污染历史 (Priority: P2)

**Goal**: 让父 turn 结束后的子交付通过运行时信号和一次性交付输入继续推进父智能体，而不把机制消息写进 durable 历史。

**Independent Test**: 让多个子智能体在父 turn 结束后依次交付结果，验证父智能体都能被运行时唤醒继续处理，且父 durable 历史只保留边界事实。

### Tests for User Story 5

- [x] T035 [P] [US5] Add runtime regression tests for post-turn parent wake-up, one-shot consumption, and multi-delivery buffering in `crates/runtime/src/service/execution/tests.rs` and `crates/runtime-agent-control/src/lib.rs`
- [x] T036 [P] [US5] Add server/frontend projection tests proving durable delivery facts remain traceable without `ReactivationPrompt` messages in `crates/server/src/tests/session_contract_tests.rs`, `frontend/src/lib/sessionHistory.test.ts`, and `frontend/src/lib/agentEvent.test.ts`

### Implementation for User Story 5

- [x] T037 [US5] Replace durable `ReactivationPrompt` writes with runtime wake signals and ephemeral delivery declarations in `crates/runtime-agent-loop/src/subagent.rs`, `crates/runtime/src/service/execution/mod.rs`, and `crates/runtime/src/service/execution/collaboration.rs`
- [x] T038 [US5] Add buffered parent-delivery queueing, dedupe, and busy-parent drain logic in `crates/runtime-agent-control/src/lib.rs`, `crates/runtime/src/service/execution/status.rs`, and `crates/runtime/src/service/execution/cancel.rs`
- [x] T039 [US5] Keep durable delivery facts and child entry links while excluding mechanism messages from replay and projections in `crates/core/src/event/translate.rs`, `crates/core/src/projection/agent_state.rs`, and `crates/server/src/http/routes/sessions/query.rs`
- [x] T040 [US5] Surface cancelled or terminated child summaries and default parent summary projections in `crates/runtime/src/service/execution/status.rs`, `frontend/src/lib/subRunView.ts`, `frontend/src/components/Chat/SubRunBlock.tsx`, and `frontend/src/App.tsx`

**Checkpoint**: 父 turn 结束后的子交付可以可靠排队和逐个消费，父 durable 历史不再包含机制性唤醒消息。

---

## Phase 8: Polish & Cross-Cutting Concerns

**Purpose**: 做最后的死代码清理、文档同步、健壮性审查和全量验证

- [x] T041 [P] Refresh feature docs, contracts, and cutover notes after implementation in `specs/004-subagent-cache-context/findings.md`, `specs/004-subagent-cache-context/design-runtime-boundaries.md`, `specs/004-subagent-cache-context/design-prompt-cache-and-context.md`, `specs/004-subagent-cache-context/migration.md`, and `specs/004-subagent-cache-context/quickstart.md`
- [x] T042 Remove leftover shared-session compatibility and mixed-thread-view dead code in `crates/runtime-execution/src/policy.rs`, `crates/runtime-session/src/session_state.rs`, `frontend/src/types.ts`, `frontend/src/store/reducer.ts`, and `frontend/src/lib/subRunView.ts`
- [x] T043 Review queue and replay code for lock-held-await, unmanaged spawns, and panic paths in `crates/runtime-agent-control/src/lib.rs`, `crates/runtime-execution/src/prep.rs`, `crates/runtime-agent-loop/src/subagent.rs`, and `crates/runtime-agent-loop/src/agent_loop.rs`
- [x] T044 Review observability and error propagation for child lifecycle, cache invalidation, lineage mismatch, delivery buffering, and legacy rejection in `crates/runtime/src/service/observability.rs`, `crates/runtime-execution/src/lib.rs`, and `crates/server/src/http/routes/sessions/stream.rs`
- [x] T045 Run the repository validation matrix from `specs/004-subagent-cache-context/quickstart.md`

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1: Setup** has no dependencies and can start immediately.
- **Phase 2: Foundational** depends on Phase 1 and blocks all user stories.
- **Phase 3: US1** depends on Phase 2 and is the MVP release slice.
- **Phase 4: US2** depends on US1 because replay-based resume needs the stable child-session truth from US1.
- **Phase 5: US3** depends on US1 because structured inheritance must target the independent child-session model introduced there.
- **Phase 6: US4** depends on US3 because shared cache boundaries rely on inherited prompt blocks and task or background separation.
- **Phase 7: US5** depends on US1 and US3 because runtime wake-up reuses stable child identities and one-shot prompt-side delivery input.
- **Phase 8: Polish** depends on all desired stories being complete.

### User Story Dependencies

- **US1 (P1)**: First deliverable and recommended MVP.
- **US2 (P1)**: Builds on US1 stable child-session durable truth.
- **US3 (P2)**: Builds on US1 and prepares the prompt-side split that later stories reuse.
- **US4 (P2)**: Builds on US3 structured inherited context and shared prompt-builder ownership.
- **US5 (P2)**: Builds on US1 lifecycle identity plus US3 prompt-side ephemeral input support.

### Within Each User Story

- Tests must exist before the story is considered complete.
- Durable model and event or protocol updates precede runtime orchestration changes.
- Runtime orchestration precedes server projections.
- Server projections precede frontend read-model updates.
- Observability and error-context tasks finish before story sign-off.

### Parallel Opportunities

- `T002` and `T003` can run in parallel after `T001`.
- `T005`, `T006`, and `T007` can run in parallel after `T004`.
- `T009` and `T010` can run in parallel for US1.
- `T017` and `T018` can run in parallel for US2.
- `T023` and `T024` can run in parallel for US3.
- `T029` and `T030` can run in parallel for US4.
- `T035` and `T036` can run in parallel for US5.
- `T041` and `T044` can run in parallel after feature code paths are complete.

---

## Parallel Example: User Story 1

```bash
# Launch the US1 proof tasks together
Task: "Add runtime regression tests for independent child-session spawn, parent-history cleanliness, and legacy shared-history rejection in crates/runtime/src/service/execution/tests.rs and crates/runtime-execution/src/subrun.rs"
Task: "Add server/frontend projection tests proving parent history exposes only summary facts and child entry links in crates/server/src/tests/session_contract_tests.rs, frontend/src/lib/sessionHistory.test.ts, and frontend/src/lib/subRunView.test.ts"
```

## Parallel Example: User Story 3

```bash
# Validate message/prompt separation and inherited-tail safety together
Task: "Add prompt/runtime tests proving child task messages stay task-only while inherited blocks render in LayeredPromptBuilder.Inherited between SemiStable and Dynamic in crates/runtime-agent-loop/src/agent_loop/tests/prompt.rs and crates/runtime/src/service/execution/tests.rs"
Task: "Add regression tests for deterministic recent-tail clipping and zero durable UserMessage leakage from inherited context in crates/runtime-agent-loop/src/context_window/compaction.rs and crates/core/src/event/translate.rs"
```

## Parallel Example: User Story 5

```bash
# Split wake-up buffering proof and projection proof
Task: "Add runtime regression tests for post-turn parent wake-up, one-shot consumption, and multi-delivery buffering in crates/runtime/src/service/execution/tests.rs and crates/runtime-agent-control/src/lib.rs"
Task: "Add server/frontend projection tests proving durable delivery facts remain traceable without ReactivationPrompt messages in crates/server/src/tests/session_contract_tests.rs, frontend/src/lib/sessionHistory.test.ts, and frontend/src/lib/agentEvent.test.ts"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup.
2. Complete Phase 2: Foundational.
3. Complete Phase 3: US1.
4. Run the US1-specific tests and targeted validation from `specs/004-subagent-cache-context/quickstart.md`.
5. Demo the independent child-session durable model before taking on replay, prompt, cache, or wake-up refinements.

### Incremental Delivery

1. Finish Setup + Foundational once.
2. Deliver US1 for durable child-session truth and legacy rejection.
3. Deliver US2 for replay-based resume and lineage failure exposure.
4. Deliver US3 for structured inherited context.
5. Deliver US4 for shared cache reuse and observability.
6. Deliver US5 for runtime wake-up and delivery buffering.
7. Finish with Polish and the full validation matrix.

### Parallel Team Strategy

1. One engineer owns Phase 2 durable contracts, docs cutover, and event or protocol groundwork.
2. After Phase 2, one engineer can take US1 runtime persistence while another prepares US1 projection updates.
3. Once US1 lands, resume work (US2) and prompt/context work (US3) can proceed in parallel with careful file ownership.
4. Cache reuse (US4) should wait until US3 has stabilized the inherited-block boundary.
5. Wake-up buffering (US5) should wait until US1 and US3 have stabilized identity and prompt-side delivery input.

---

## Notes

- `runtime-prompt` remains the only source of truth for prompt fingerprint behavior.
- `ReactivationPrompt` must disappear from the parent durable history path, not merely be hidden in projection.
- Legacy shared-write histories are not supported inputs and must return `unsupported_legacy_shared_history`.
- `SubRun` remains the child execution-instance concept; `ChildSessionNode` remains the durable child-session concept.
- 每个故事完成后都应单独执行其独立验收场景，而不是只等到最终全量验证。
