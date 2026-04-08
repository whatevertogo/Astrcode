---

description: "Task list for 子 Agent Child Session 与协作工具重构"
---

# Tasks: 子 Agent Child Session 与协作工具重构

**Input**: Design documents from `/specs/003-subagent-child-sessions/`
**Prerequisites**: `plan.md`, `spec.md`, `research.md`, `data-model.md`, `contracts/`, `quickstart.md`, `migration.md`

**Tests**: 本特性会修改 durable event、protocol DTO、tool/capability 边界、runtime 生命周期与前端主视图，因此测试任务为强制项，不允许省略。

**Organization**: Tasks are grouped by user story so each story can be implemented, tested, and validated independently.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel after prerequisite dependencies are satisfied
- **[Story]**: Which user story this task belongs to (`US1`-`US5`)
- Every task includes exact file paths

## Path Conventions

- Rust backend crates live under `crates/`
- HTTP/SSE DTOs and transport contracts live under `crates/protocol/` and `crates/server/`
- Frontend state, API clients, and chat UI live under `frontend/src/`
- Feature docs and validation commands live under `specs/003-subagent-child-sessions/`

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Establish shared fixtures, entry points, and scaffolding before the durable refactor starts

- [X] T001 Create shared child-session backend test fixtures in `crates/core/src/test_support.rs` and `crates/runtime-agent-loop/src/agent_loop/tests/fixtures.rs`
- [X] T002 [P] Create frontend child-session projection fixtures in `frontend/src/lib/agentEvent.test.ts` and `frontend/src/lib/subRunView.test.ts`
- [X] T003 [P] Wire child-session module entry points in `crates/runtime/src/service/execution/mod.rs`, `crates/runtime-execution/src/lib.rs`, and `crates/runtime-session/src/lib.rs`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core contracts and runtime boundaries that MUST be complete before any user story can ship

**⚠️ CRITICAL**: No user story work should start until this phase is complete

- [X] T004 Define core child-session domain types and exports in `crates/core/src/agent/mod.rs` and `crates/core/src/lib.rs`
- [ ] T005 [P] Add durable child-session storage events and translation support in `crates/core/src/event/domain.rs`, `crates/core/src/event/types.rs`, and `crates/core/src/event/translate.rs`
- [ ] T006 [P] Add protocol DTOs and serialization for child-session status, notifications, and view projections in `crates/protocol/src/http/agent.rs`, `crates/protocol/src/http/event.rs`, `crates/protocol/src/http/session.rs`, and `crates/protocol/src/http/mod.rs`
- [X] T007 [P] Move runtime-only capability/tool context defaults into `crates/runtime-registry/src/tool.rs` and `crates/runtime-registry/src/router.rs`
- [X] T008 Refactor `ToolRegistry` into test/build helper while `CapabilityRouter` remains the only production entry in `crates/runtime-registry/src/tool.rs`, `crates/runtime-registry/src/router.rs`, and `crates/core/src/registry/router.rs`
- [X] T009 [P] Remove `runtime-agent-tool` as the semantic source of `AgentProfileCatalog` in `crates/runtime-agent-loop/src/prompt_runtime.rs`, `crates/runtime-prompt/src/contributors/agent_profile_summary.rs`, and `crates/runtime-agent-tool/src/lib.rs`
- [X] T010 Add foundational round-trip coverage for durable child-session contracts in `crates/core/src/event/types.rs`, `crates/core/src/event/translate.rs`, and `crates/protocol/tests/subrun_event_serialization.rs`

**Checkpoint**: Foundation ready. Durable truth, protocol contracts, and registry ownership are stable enough for story work.

---

## Phase 3: User Story 1 - 稳定委派与交付 (Priority: P1) 🎯 MVP

**Goal**: Let child agents live as durable child sessions that survive parent-turn completion and deliver a single structured terminal result back to the parent.

**Independent Test**: Start a long-running child agent, let the parent turn finish first, and verify the child continues running, produces exactly one terminal delivery, and can reopen the same child session by stable identity.

### Tests for User Story 1

- [ ] T011 [P] [US1] Add regression tests for parent-turn completion and single terminal delivery in `crates/runtime/src/service/execution/tests.rs` and `crates/runtime-agent-loop/src/agent_loop/tests/tool_execution.rs`
- [ ] T012 [P] [US1] Add server contract tests for child-session status source and final delivery projection in `crates/server/src/tests/session_contract_tests.rs`

### Implementation for User Story 1

- [ ] T013 [US1] Persist `ChildSessionNode` and execution boundary data on spawn in `crates/runtime/src/service/execution/subagent.rs` and `crates/runtime-session/src/session_state.rs`
- [ ] T014 [US1] Stop parent-turn cleanup from cancelling durable child sessions in `crates/runtime/src/service/turn/orchestration.rs` and `crates/runtime-agent-control/src/lib.rs`
- [ ] T015 [US1] Build durable child delivery notifications with final-reply and failure fallback in `crates/runtime-execution/src/subrun.rs` and `crates/runtime/src/service/execution/status.rs`
- [ ] T016 [US1] Reactivate the parent agent when child delivery arrives after the parent turn ended in `crates/runtime-agent-loop/src/agent_loop.rs`, `crates/runtime-agent-loop/src/subagent.rs`, and `crates/runtime/src/service/execution/mod.rs`
- [ ] T017 [US1] Return stable `ChildAgentRef` and `openSessionId` metadata from `spawnAgent` in `crates/runtime-agent-tool/src/spawn_tool.rs` and `crates/runtime-agent-tool/src/result_mapping.rs`
- [ ] T018 [US1] Add structured error context and observability for child-session creation and terminal delivery in `crates/runtime/src/service/execution/subagent.rs` and `crates/runtime-execution/src/lib.rs`

**Checkpoint**: Child sessions survive parent-turn completion and produce one durable, consumable delivery to the parent.

---

## Phase 4: User Story 2 - 可查看的子会话视图 (Priority: P1)

**Goal**: Replace mixed-session reconstruction with a parent summary list plus direct child-session viewing, showing thinking, tool activity, and final reply without raw JSON.

**Independent Test**: Run one successful child and one failed child, open each from the parent summary list after reload, and verify both show readable timelines without exposing raw protocol JSON.

### Tests for User Story 2

- [ ] T019 [P] [US2] Add server projection tests for parent summary lists and direct child-session loading in `crates/server/src/tests/session_contract_tests.rs`
- [ ] T020 [P] [US2] Add frontend tests for summary cards, single-child expansion, and hidden raw JSON in `frontend/src/lib/subRunView.test.ts` and `frontend/src/components/Chat/SubRunBlock.test.tsx`

### Implementation for User Story 2

- [ ] T021 [US2] Add child summary and child-session route mappers in `crates/protocol/src/http/session.rs`, `crates/server/src/http/routes/sessions/query.rs`, and `crates/server/src/http/routes/sessions/stream.rs`
- [ ] T022 [US2] Add frontend API client support for child summary lists and direct child-session loading in `frontend/src/lib/api/models.ts` and `frontend/src/lib/api/sessions.ts`
- [ ] T023 [US2] Replace mixed-session tree building with parent-summary projection helpers in `frontend/src/lib/subRunView.ts` and `frontend/src/lib/sessionView.ts`
- [ ] T024 [US2] Update session orchestration to preserve active child-session identity across refresh in `frontend/src/hooks/useAgent.ts`, `frontend/src/hooks/useAgentEventHandler.ts`, and `frontend/src/App.tsx`
- [ ] T025 [US2] Redesign the child-session block as collapsible thinking, tool-activity, and final-reply sections in `frontend/src/components/Chat/SubRunBlock.tsx` and `frontend/src/components/Chat/SubRunBlock.module.css`
- [ ] T026 [US2] Remove default raw JSON rendering from child-session views in `frontend/src/components/Chat/ToolJsonView.tsx` and `frontend/src/components/Chat/AssistantMessage.tsx`

**Checkpoint**: The parent view uses summaries only, and every child can be reopened directly as its own readable session.

---

## Phase 5: User Story 3 - 主子双向协作 (Priority: P2)

**Goal**: Let parent and child agents keep collaborating through explicit tools (`send`, `wait`, `close`, `resume`, `deliver`) without creating duplicate child sessions.

**Independent Test**: Create one child, let it finish an initial answer, send a follow-up revision request to the same child, wait on only that child, and verify the reused session produces a second delivery without affecting sibling children.

### Tests for User Story 3

- [ ] T027 [P] [US3] Add tool contract tests for `sendAgent`, `waitAgent`, `closeAgent`, `resumeAgent`, and `deliverToParent` in `crates/runtime-agent-tool/src/tests.rs`
- [ ] T028 [P] [US3] Add runtime tests for targeted wait, resume, close, and single-consume delivery handling in `crates/runtime/src/service/execution/tests.rs` and `crates/runtime-agent-control/src/lib.rs`

### Implementation for User Story 3

- [ ] T029 [US3] Define collaboration tool params, results, and capability descriptors in `crates/core/src/agent/mod.rs`, `crates/protocol/src/capability/descriptors.rs`, and `crates/protocol/src/http/tool.rs`
- [ ] T030 [US3] Implement `sendAgent`, `waitAgent`, `closeAgent`, `resumeAgent`, and `deliverToParent` adapters in `crates/runtime-agent-tool/src/lib.rs`, `crates/runtime-agent-tool/src/result_mapping.rs`, `crates/runtime-agent-tool/src/send_tool.rs`, `crates/runtime-agent-tool/src/wait_tool.rs`, `crates/runtime-agent-tool/src/close_tool.rs`, `crates/runtime-agent-tool/src/resume_tool.rs`, and `crates/runtime-agent-tool/src/deliver_tool.rs`
- [ ] T031 [US3] Add durable inbox and mailbox enqueue plus dedupe handling for collaboration requests in `crates/runtime-execution/src/context.rs`, `crates/runtime-execution/src/lib.rs`, and `crates/runtime-session/src/turn_runtime.rs`
- [ ] T032 [US3] Reuse the same child session on resume and follow-up requests in `crates/runtime/src/service/execution/subagent.rs` and `crates/runtime/src/service/execution/status.rs`
- [ ] T033 [US3] Register collaboration tools only through `CapabilityRouter` in `crates/runtime-registry/src/router.rs`, `crates/runtime/src/service/capability_manager.rs`, and `crates/runtime/src/service/execution/surface.rs`
- [ ] T034 [US3] Surface the collaboration tool family and stable child refs in prompt assembly without recoupling to tool internals in `crates/runtime-prompt/src/contributors/capability_prompt.rs` and `crates/runtime-agent-loop/src/prompt_runtime.rs`

**Checkpoint**: Parent and child agents can keep collaborating through one stable child session identity and one tool surface.

---

## Phase 6: User Story 4 - 层级协作与级联关闭 (Priority: P2)

**Goal**: Make collaboration and shutdown follow the durable agent ownership tree, including direct-parent-only routing and leaf-first cascade close semantics.

**Independent Test**: Build a three-level agent chain, send an upward delivery from the deepest child to its direct parent, then close the middle agent and verify only that subtree closes from leaves upward while sibling branches stay alive.

### Tests for User Story 4

- [ ] T035 [P] [US4] Add hierarchy regression tests for leaf-first cascade, subtree isolation, and direct-parent-only delivery in `crates/runtime-agent-control/src/lib.rs` and `crates/runtime/src/service/execution/tests.rs`
- [ ] T036 [P] [US4] Add agent-loop behavior tests for close-or-keep decisions after child delivery in `crates/runtime-agent-loop/src/agent_loop/tests/regression.rs` and `crates/runtime-agent-loop/src/agent_loop/tests/tool_execution.rs`

### Implementation for User Story 4

- [ ] T037 [US4] Persist agent ownership-tree traversal helpers alongside durable child-session nodes in `crates/runtime-agent-control/src/lib.rs` and `crates/runtime-session/src/session_state.rs`
- [ ] T038 [US4] Execute close propagation by agent subtree instead of parent turn in `crates/runtime-agent-control/src/lib.rs` and `crates/runtime/src/service/execution/cancel.rs`
- [ ] T039 [US4] Enforce direct-parent routing and one-time consumption for upward deliveries in `crates/runtime-execution/src/lib.rs` and `crates/runtime-session/src/turn_runtime.rs`
- [ ] T040 [US4] Append default close-or-keep child guidance to parent follow-up input assembly in `crates/runtime-agent-loop/src/request_assembler.rs`, `crates/runtime-agent-loop/src/prompt_runtime.rs`, and `crates/runtime-prompt/src/contributors/workflow_examples.rs`

**Checkpoint**: Collaboration routes by durable ownership, and close propagation is subtree-scoped and leaf-first.

---

## Phase 7: User Story 5 - 为未来 Fork Agent 复用同一底座 (Priority: P3)

**Goal**: Keep fork-agent support on the same lifecycle, projection, and delivery model by adding lineage metadata instead of creating a second system.

**Independent Test**: Validate that `spawn`, `fork`, and `resume` child refs all flow through the same child-session status and view pipeline, differing only by lineage metadata.

### Tests for User Story 5

- [ ] T041 [P] [US5] Add lineage compatibility tests for `spawn`, `fork`, and `resume` child refs plus status projections in `crates/core/src/event/types.rs`, `crates/protocol/tests/subrun_event_serialization.rs`, and `crates/server/src/tests/session_contract_tests.rs`

### Implementation for User Story 5

- [ ] T042 [US5] Add lineage snapshot metadata to durable child-session records in `crates/core/src/agent/mod.rs`, `crates/runtime-session/src/session_state.rs`, and `crates/runtime-execution/src/subrun.rs`
- [ ] T043 [US5] Propagate lineage kind through server and frontend read models without creating a second lifecycle system in `crates/protocol/src/http/agent.rs`, `frontend/src/lib/api/models.ts`, and `frontend/src/lib/subRunView.ts`
- [ ] T044 [US5] Reuse existing child-session orchestration entry points for future fork creation in `crates/runtime/src/service/execution/context.rs` and `crates/runtime/src/service/execution/subagent.rs`

**Checkpoint**: Fork lineage is a metadata extension on the same child-session model, not a parallel subsystem.

---

## Phase 8: Polish & Cross-Cutting Concerns

**Purpose**: Final cleanup, observability hardening, documentation sync, and repository-wide validation

- [ ] T045 [P] Update final caller migration and validation docs in `specs/003-subagent-child-sessions/findings.md`, `specs/003-subagent-child-sessions/design-collaboration-runtime.md`, `specs/003-subagent-child-sessions/design-parent-child-projection.md`, `specs/003-subagent-child-sessions/migration.md`, and `specs/003-subagent-child-sessions/quickstart.md`
- [ ] T046 Remove legacy mixed-session and subrun-only production paths from `frontend/src/lib/subRunView.ts`, `frontend/src/lib/api/sessions.ts`, `crates/server/src/http/routes/sessions/mutation.rs`, and `crates/runtime-registry/src/tool.rs`
- [ ] T047 Add cross-cutting observability and error-context review for collaboration delivery, reactivation, and child-session projection failures in `crates/runtime-execution/src/lib.rs`, `crates/runtime/src/service/execution/subagent.rs`, and `crates/server/src/http/routes/sessions/stream.rs`
- [ ] T048 Run the full repository validation matrix documented in `specs/003-subagent-child-sessions/quickstart.md`

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1: Setup** has no dependencies and can start immediately.
- **Phase 2: Foundational** depends on Phase 1 and blocks all story work.
- **Phase 3: US1** depends on Phase 2 and is the MVP release slice.
- **Phase 4: US2** depends on US1 because the parent summary and child-session UI need stable durable child identities and delivery projections.
- **Phase 5: US3** depends on US1 because follow-up collaboration must reuse the durable child-session model introduced there.
- **Phase 6: US4** depends on US3 because hierarchy-aware close and direct-parent routing extend the collaboration tool flow.
- **Phase 7: US5** depends on US1 and US4 because fork reuse is only meaningful once lifecycle, routing, and ownership-tree rules are already unified.
- **Phase 8: Polish** depends on all desired stories being complete.

### User Story Dependencies

- **US1 (P1)**: First deliverable and recommended MVP.
- **US2 (P1)**: Builds on US1 durable session identity and delivery summaries.
- **US3 (P2)**: Builds on US1 stable child-session lifecycle and extends tool contracts.
- **US4 (P2)**: Builds on US3 collaboration tools plus durable ownership-tree semantics.
- **US5 (P3)**: Builds on the shared lifecycle from US1-US4 and adds lineage reuse.

### Within Each User Story

- Tests must exist before the story is considered complete.
- Durable model and protocol updates precede runtime orchestration changes.
- Runtime orchestration precedes server projections.
- Server projections precede frontend read-model and UI rewrites.
- Prompt/runtime guidance changes land after the underlying collaboration behavior is stable.

### Parallel Opportunities

- `T002` and `T003` can run in parallel after `T001`.
- `T005`, `T006`, `T007`, and `T009` can run in parallel after `T004`.
- `T011` and `T012` can run in parallel for US1.
- `T019` and `T020` can run in parallel for US2.
- `T027` and `T028` can run in parallel for US3.
- `T035` and `T036` can run in parallel for US4.
- `T041` can run while US5 implementation planning starts because it only locks lineage expectations.
- `T045` and `T047` can run in parallel after all feature code paths are complete.

---

## Parallel Example: User Story 1

```bash
# Launch the two US1 proof tasks together
Task: "Add regression tests for parent-turn completion and single terminal delivery in crates/runtime/src/service/execution/tests.rs and crates/runtime-agent-loop/src/agent_loop/tests/tool_execution.rs"
Task: "Add server contract tests for child-session status source and final delivery projection in crates/server/src/tests/session_contract_tests.rs"
```

## Parallel Example: User Story 2

```bash
# Validate parent summary projection and UI behavior together
Task: "Add server projection tests for parent summary lists and direct child-session loading in crates/server/src/tests/session_contract_tests.rs"
Task: "Add frontend tests for summary cards, single-child expansion, and hidden raw JSON in frontend/src/lib/subRunView.test.ts and frontend/src/components/Chat/SubRunBlock.test.tsx"
```

## Parallel Example: User Story 3

```bash
# Split contract-level and runtime-level collaboration verification
Task: "Add tool contract tests for sendAgent, waitAgent, closeAgent, resumeAgent, and deliverToParent in crates/runtime-agent-tool/src/tests.rs"
Task: "Add runtime tests for targeted wait, resume, close, and single-consume delivery handling in crates/runtime/src/service/execution/tests.rs and crates/runtime-agent-control/src/lib.rs"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup.
2. Complete Phase 2: Foundational.
3. Complete Phase 3: US1.
4. Run the US1-specific tests and targeted validation from `specs/003-subagent-child-sessions/quickstart.md`.
5. Demo the stable durable child-session lifecycle before taking on collaboration or UI rewrites.

### Incremental Delivery

1. Finish Setup + Foundational once.
2. Deliver US1 for durable lifecycle and parent reactivation.
3. Deliver US2 for readable parent/child viewing without raw JSON.
4. Deliver US3 for explicit collaboration tools.
5. Deliver US4 for hierarchy routing and cascade close.
6. Deliver US5 for fork-ready lineage reuse.
7. Finish with Polish and the full validation matrix.

### Parallel Team Strategy

1. One engineer owns Phase 2 durable contracts and registry boundaries.
2. One engineer can take US1 runtime orchestration while another prepares US2 projection tests after Phase 2 lands.
3. After US1 stabilizes, collaboration tooling (US3) and UI projection cleanup (US2) can overlap with careful file ownership.
4. Hierarchy work (US4) should wait until collaboration routing is stable.

---

## Notes

- `CapabilityRouter` remains the only production execution registry throughout implementation.
- `ToolRegistry` is allowed only as a test/build helper after the refactor.
- Parent view must never become the durable truth for child sessions again.
- Raw JSON stays available only for debugging paths, not for the default user-facing child-session UI.
