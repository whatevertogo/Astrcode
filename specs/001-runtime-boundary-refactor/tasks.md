# Tasks: Runtime Boundary Refactor

**Input**: Design documents from `/specs/001-runtime-boundary-refactor/`  
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md

**Tests**: Validation tasks are mandatory for this feature because it changes durable events, replay semantics, runtime boundaries, and public server/frontend surfaces.

**Organization**: Tasks are grouped by user story so each story remains independently implementable and testable.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependency on unfinished tasks)
- **[Story]**: Which user story this task belongs to (`[US1]`, `[US2]`, `[US3]`)
- Every task includes exact file paths in the description

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Freeze the migration baseline, caller inventory, and validation targets before code movement starts

- [X] T001 Reconcile the caller inventory and deletion order in `specs/001-runtime-boundary-refactor/migration.md` against `crates/runtime/src/service/mod.rs`, `crates/server/src/http/routes/sessions/query.rs`, `crates/server/src/http/routes/sessions/stream.rs`, `crates/server/src/http/routes/sessions/mutation.rs`, and `crates/server/src/http/routes/agents.rs`
- [X] T002 Capture the validation matrix and legacy-history sample coverage in `specs/001-runtime-boundary-refactor/quickstart.md`, `crates/protocol/tests/fixtures/`, `crates/server/src/tests/runtime_routes_tests.rs`, and `crates/server/src/tests/session_contract_tests.rs`
- [X] T003 [P] Align implementation checklists in `specs/001-runtime-boundary-refactor/design-subrun-protocol.md`, `specs/001-runtime-boundary-refactor/design-execution-boundary.md`, and `specs/001-runtime-boundary-refactor/contracts/`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Establish shared types, contracts, and crate wiring required by every story

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [X] T004 Add shared lineage value objects in `crates/core/src/agent/mod.rs` and `crates/core/src/lib.rs`
- [X] T005 Update shared durable/domain event scaffolding in `crates/core/src/event/types.rs`, `crates/core/src/event/domain.rs`, and `crates/core/src/event/translate.rs`
- [X] T006 Define protocol DTO shells and mapper/front-end type placeholders in `crates/protocol/src/http/event.rs`, `crates/protocol/src/http/agent.rs`, `crates/server/src/http/mapper.rs`, and `frontend/src/types.ts`
- [X] T007 Define cross-boundary runtime traits for session truth, execution orchestration, loop running, and live subrun control in `crates/core/src/runtime/traits.rs`, `crates/core/src/runtime/mod.rs`, and `crates/core/src/lib.rs`
- [X] T008 Configure the target owner graph in `crates/runtime-session/Cargo.toml`, `crates/runtime-execution/Cargo.toml`, `crates/runtime/Cargo.toml`, and `crates/runtime/src/service/mod.rs`

**Checkpoint**: Foundation ready — durable lineage, protocol surfaces, core traits, and boundary wiring are defined for implementation

---

## Phase 3: User Story 1 - Durable Child Execution Truth (Priority: P1) 🎯 MVP

**Goal**: Make durable subrun lineage and trigger facts survive beyond live runtime state

**Independent Test**: Start a child subrun, let it finish, clear live state, then verify `/history`, `/events`, and subrun status still reconstruct the same descriptor and trigger source

### Tests for User Story 1 ⚠️

> **NOTE: Write these tests first and confirm they fail before implementation**

- [X] T009 [P] [US1] Extend durable event serialization coverage in `crates/protocol/tests/subrun_event_serialization.rs` and `crates/protocol/tests/conformance.rs`
- [X] T010 [P] [US1] Add durable/live status reconstruction tests in `crates/runtime-execution/src/subrun.rs` and `crates/runtime/src/service/execution/tests.rs`
- [X] T011 [P] [US1] Add server subrun status contract coverage in `crates/server/src/tests/runtime_routes_tests.rs` and `crates/server/src/tests/session_contract_tests.rs`
- [X] T012 [P] [US1] Add parent-termination and storage-mode parity regressions in `crates/runtime-execution/src/subrun.rs`, `crates/runtime/src/service/execution/tests.rs`, and `crates/server/src/tests/runtime_routes_tests.rs`

### Implementation for User Story 1

- [X] T013 [US1] Persist `descriptor` and `tool_call_id` on lifecycle writes in `crates/runtime/src/service/execution/subagent.rs` and `crates/runtime/src/service/execution/status.rs`
- [X] T014 [US1] Finalize `SubRunStarted` and `SubRunFinished` durable/domain schemas in `crates/core/src/event/types.rs`, `crates/core/src/event/domain.rs`, and `crates/core/src/event/translate.rs`
- [X] T015 [US1] Rebuild durable-first replay and status lookup for completed, cancelled, and parent-aborted child executions in `crates/runtime-execution/src/subrun.rs` and `crates/runtime-execution/src/lib.rs`
- [X] T016 [US1] Expose `descriptor`, `toolCallId`, and `source` through `crates/protocol/src/http/event.rs`, `crates/protocol/src/http/agent.rs`, `crates/server/src/http/mapper.rs`, and `crates/server/src/http/routes/agents.rs`
- [X] T017 [US1] Update frontend subrun payload normalization in `frontend/src/types.ts`, `frontend/src/lib/agentEvent.ts`, and `frontend/src/lib/api/models.ts`
- [X] T018 [US1] Preserve identical ownership semantics for `SharedSession` and `IndependentSession` execution paths in `crates/runtime/src/service/execution/subagent.rs`, `crates/runtime-execution/src/subrun.rs`, `crates/core/src/agent/mod.rs`, and `crates/server/src/http/routes/agents.rs`
- [X] T019 [US1] Implement `legacyDurable` downgrade handling in `crates/runtime-execution/src/subrun.rs`, `crates/server/src/http/mapper.rs`, `frontend/src/lib/agentEvent.ts`, and `frontend/src/lib/subRunView.ts`

**Checkpoint**: User Story 1 is complete when durable history alone can answer subrun lineage and trigger questions after live cleanup, parent teardown, and storage-mode changes

---

## Phase 4: User Story 2 - Clear Runtime Responsibilities (Priority: P2)

**Goal**: Give session truth, execution orchestration, live control, and runtime facade one owner each with one public surface each

**Independent Test**: Review the boundary design and compile the workspace to verify every caller can point to a single owner surface and the legacy facades have a documented removal path

### Tests for User Story 2 ⚠️

- [X] T020 [P] [US2] Add execution-surface regression coverage in `crates/runtime/src/service/execution/tests.rs`, `crates/runtime/src/service/turn/tests.rs`, and `crates/server/src/tests/runtime_routes_tests.rs`
- [X] T021 [P] [US2] Add boundary/dependency smoke coverage in `crates/runtime-session/src/lib.rs`, `crates/runtime-session/src/turn_runtime.rs`, and `crates/runtime-execution/src/lib.rs`

### Implementation for User Story 2

- [X] T022 [US2] Strip execution orchestration out of the session boundary in `crates/runtime-session/src/lib.rs`, `crates/runtime-session/src/turn_runtime.rs`, and `crates/runtime-session/src/session_state.rs`
- [X] T023 [US2] Move submit/interrupt/root-execute/subrun orchestration into `crates/runtime-execution/src/lib.rs`, `crates/runtime-execution/src/context.rs`, `crates/runtime-execution/src/prep.rs`, and `crates/runtime-execution/src/subrun.rs`
- [X] T024 [US2] Implement the core runtime traits from `crates/core/src/runtime/traits.rs` in `crates/runtime-agent-loop/src/agent_loop/turn_runner.rs`, `crates/runtime-agent-control/src/lib.rs`, `crates/runtime-session/src/lib.rs`, and `crates/runtime/src/service/execution/mod.rs`
- [X] T025 [US2] Refactor the runtime facade to expose only owner handles in `crates/runtime/src/service/mod.rs`, `crates/runtime/src/service/execution/mod.rs`, and `crates/runtime/src/runtime_governance.rs`
- [X] T026 [US2] Migrate server and internal callers to `sessions()` / `execution()` / `tools()` in `crates/server/src/http/routes/sessions/query.rs`, `crates/server/src/http/routes/sessions/stream.rs`, `crates/server/src/http/routes/sessions/mutation.rs`, `crates/server/src/http/routes/agents.rs`, `crates/server/src/http/routes/tools.rs`, `crates/runtime/src/service/session/create.rs`, and `crates/runtime/src/service/session/delete.rs`
- [X] T027 [US2] Delete legacy facades in `crates/runtime/src/service/session_service.rs`, `crates/runtime/src/service/execution_service.rs`, `crates/runtime/src/service/replay.rs`, `crates/runtime/src/service/turn/submit.rs`, and `crates/runtime/src/service/session/load.rs`
- [X] T028 [US2] Sync final owner and deletion decisions in `specs/001-runtime-boundary-refactor/design-execution-boundary.md`, `specs/001-runtime-boundary-refactor/migration.md`, and `docs/architecture/architecture.md`

**Checkpoint**: User Story 2 is complete when there is one owner per boundary, one public surface per responsibility, and no remaining legacy facade callers

---

## Phase 5: User Story 3 - Consistent Query and Scope Behavior (Priority: P3)

**Goal**: Make history replay, incremental events, scope filtering, and frontend subrun trees share the same durable lineage semantics

**Independent Test**: Run history replay, filtered SSE replay, and scope queries over the same nested sample and confirm `self`, `directChildren`, and `subtree` all match the same lineage index; also verify working-dir resolution follows the request context

### Tests for User Story 3 ⚠️

- [X] T029 [P] [US3] Add scope-filter contract coverage in `crates/server/src/tests/runtime_routes_tests.rs` and `crates/server/src/tests/session_contract_tests.rs`
- [X] T030 [P] [US3] Add frontend lineage/tree regression coverage in `frontend/src/lib/subRunView.test.ts`, `frontend/src/lib/sessionHistory.test.ts`, `frontend/src/lib/agentEvent.test.ts`, and `frontend/src/components/Chat/SubRunBlock.test.tsx`
- [X] T031 [P] [US3] Add working-dir resolver regression coverage in `crates/runtime-agent-loader/src/lib.rs`, `crates/runtime/src/bootstrap.rs`, and `crates/runtime/src/service/watch_ops.rs`

### Implementation for User Story 3

- [X] T032 [US3] Implement the shared `ExecutionLineageIndex` and legacy-gap errors in `crates/runtime-execution/src/subrun.rs` and `crates/runtime-execution/src/context.rs`
- [X] T033 [US3] Replace server ancestry heuristics with lineage-based filtering in `crates/server/src/http/routes/sessions/filter.rs`, `crates/server/src/http/routes/sessions/query.rs`, and `crates/server/src/http/routes/sessions/stream.rs`
- [X] T034 [US3] Align history/event/status projections with one filter semantic in `crates/server/src/http/mapper.rs`, `crates/protocol/src/http/event.rs`, and `crates/protocol/src/http/agent.rs`
- [X] T035 [US3] Replace frontend `parentTurnId -> turn owner` inference with descriptor-based trees in `frontend/src/lib/subRunView.ts`, `frontend/src/lib/sessionHistory.ts`, and `frontend/src/components/Chat/SubRunBlock.tsx`
- [X] T036 [US3] Bind agent resolution and watch scope to execution context in `crates/runtime/src/bootstrap.rs`, `crates/runtime/src/service/watch_ops.rs`, `crates/runtime-agent-loader/src/lib.rs`, and `crates/server/src/http/routes/agents.rs`
- [X] T037 [US3] Surface lineage-gap and `legacyDurable` UI states in `frontend/src/types.ts`, `frontend/src/lib/api/models.ts`, and `frontend/src/components/Chat/SubRunBlock.tsx`

**Checkpoint**: User Story 3 is complete when history, SSE, status, scope filtering, and the frontend tree all agree on the same ownership semantics

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Final documentation sync and full validation across backend and frontend

- [ ] T038 [P] Refresh the final architecture/contracts docs in `specs/001-runtime-boundary-refactor/findings.md`, `specs/001-runtime-boundary-refactor/design-subrun-protocol.md`, `specs/001-runtime-boundary-refactor/contracts/session-history-and-events.md`, and `specs/001-runtime-boundary-refactor/contracts/execution-status-and-agent-resolution.md`
- [ ] T039 Run backend validation commands documented in `specs/001-runtime-boundary-refactor/quickstart.md`
- [ ] T040 Run frontend validation and manual acceptance scenarios documented in `specs/001-runtime-boundary-refactor/quickstart.md`

---

## Phase 7: Review all the codes

review 所有代码


## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies; start immediately
- **Foundational (Phase 2)**: Depends on Setup; blocks all story work
- **User Story 1 (Phase 3)**: Depends on Foundational; establishes durable lineage truth required by later stories
- **User Story 2 (Phase 4)**: Depends on User Story 1; boundary extraction and facade deletion assume the new protocol/status surfaces are stable
- **User Story 3 (Phase 5)**: Depends on User Story 1; task `T036` should land after User Story 2 because execution-context ownership must be stable before resolver migration completes
- **Polish (Phase 6)**: Depends on all desired user stories being complete

### User Story Dependencies

- **US1 (P1)**: No story dependency after Foundational; this is the MVP
- **US2 (P2)**: Requires US1 durable facts so caller migration and facade deletion target stable surfaces
- **US3 (P3)**: Requires US1 descriptor semantics; query convergence can begin after US1, but resolver completion should follow US2 boundary extraction

### Within Each User Story

- Write tests before the corresponding implementation tasks
- Update durable/domain types before projecting them through server/frontend surfaces
- Validate storage-mode parity before declaring durable ownership stable
- Migrate callers before deleting replaced facades
- Finish backend lineage semantics before final frontend tree/UI cleanup

### Parallel Opportunities

- `T003` can run in parallel with `T001`-`T002`
- `T009`-`T012` can run in parallel for US1
- `T020` and `T021` can run in parallel for US2
- `T029`-`T031` can run in parallel for US3
- `T038` can run in parallel with final validation once implementation is complete

---

## Parallel Example: User Story 1

```bash
# Launch the US1 test tasks together:
Task: T009
Task: T010
Task: T011
Task: T012
```

## Parallel Example: User Story 2

```bash
# Launch the US2 regression/smoke tasks together:
Task: T020
Task: T021
```

## Parallel Example: User Story 3

```bash
# Launch the US3 test tasks together:
Task: T029
Task: T030
Task: T031
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational
3. Complete Phase 3: User Story 1
4. **STOP and VALIDATE** using the durable lineage scenarios in `specs/001-runtime-boundary-refactor/quickstart.md`
5. Demo or merge the durable protocol slice before boundary extraction

### Incremental Delivery

1. Finish Setup + Foundational to freeze contracts, core traits, and owner graph
2. Deliver US1 to make durable lineage truthful across live cleanup, parent teardown, and storage modes
3. Deliver US2 to collapse duplicate runtime surfaces onto trait-backed owners
4. Deliver US3 to unify query/filter/frontend semantics and resolver scope
5. Finish Phase 6 full validation before merge

### Parallel Team Strategy

1. One owner completes Setup + Foundational
2. After US1 lands, split work by stable file sets:
   - Developer A: US2 boundary extraction and facade deletion
   - Developer B: US3 query/filter/frontend convergence
3. Rejoin for final validation and doc sync

---

## Notes

- `[P]` tasks target disjoint files and can be parallelized safely
- Story labels map every implementation task back to a single acceptance target
- Each story remains independently testable at its checkpoint
- Validation is mandatory for this feature because protocol, replay, ownership semantics, and public runtime surfaces all change
