# Tasks: Astrcode Agent 协作四工具重构

**Input**: Design documents from `D:/GitObjectsOwn/Astrcode/specs/008-agent-four-tools/`
**Prerequisites**: [plan.md](D:/GitObjectsOwn/Astrcode/specs/008-agent-four-tools/plan.md), [spec.md](D:/GitObjectsOwn/Astrcode/specs/008-agent-four-tools/spec.md), [research.md](D:/GitObjectsOwn/Astrcode/specs/008-agent-four-tools/research.md), [data-model.md](D:/GitObjectsOwn/Astrcode/specs/008-agent-four-tools/data-model.md), [contracts/agent-collaboration-tool-contract.md](D:/GitObjectsOwn/Astrcode/specs/008-agent-four-tools/contracts/agent-collaboration-tool-contract.md), [contracts/mailbox-event-contract.md](D:/GitObjectsOwn/Astrcode/specs/008-agent-four-tools/contracts/mailbox-event-contract.md), [quickstart.md](D:/GitObjectsOwn/Astrcode/specs/008-agent-four-tools/quickstart.md), [findings.md](D:/GitObjectsOwn/Astrcode/specs/008-agent-four-tools/findings.md), [design-collaboration-runtime.md](D:/GitObjectsOwn/Astrcode/specs/008-agent-four-tools/design-collaboration-runtime.md), [migration.md](D:/GitObjectsOwn/Astrcode/specs/008-agent-four-tools/migration.md)

**Tests**: 本特性必须包含行为测试和仓库级验证，因为它同时修改 durable 事件、公开 runtime surface、父子调度语义和外部调用面。

**Organization**: 任务按用户故事组织，确保 US1、US2、US3 都可以独立实现和验证。

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: 锁定迁移边界、调用方清单与最终验收基线

- [ ] T001 Lock the old-collaboration caller inventory and removal order in `specs/008-agent-four-tools/findings.md` and `specs/008-agent-four-tools/migration.md`
- [ ] T002 [P] Lock the mailbox event glossary, snapshot field sources, and replay checkpoints in `specs/008-agent-four-tools/data-model.md` and `specs/008-agent-four-tools/quickstart.md`
- [ ] T003 [P] Lock the four-tool public contract and removed-surface invariants in `specs/008-agent-four-tools/contracts/agent-collaboration-tool-contract.md` and `specs/008-agent-four-tools/contracts/mailbox-event-contract.md`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: 建立所有用户故事共享的契约、控制树和 durable mailbox 地基

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [ ] T004 Update shared agent contracts, lifecycle/outcome enums, four-tool params, `delivery_id`/`batch_id` models, and durable mailbox event payloads in `crates/core/src/agent/mod.rs`, `crates/runtime-agent-tool/src/collab_result_mapping.rs`, and `crates/runtime-agent-tool/src/result_mapping.rs`
- [ ] T005 Register root agents in the control tree and split live lifecycle/outcome state in `crates/runtime-agent-control/src/lib.rs`
- [ ] T006 [P] Add durable mailbox append/replay primitives, mailbox projector helpers, and structured error/logging hooks in `crates/runtime-session/src/session_state.rs`, `crates/runtime-session/src/turn_runtime.rs`, `crates/runtime-session/src/lib.rs`, `crates/runtime/src/service/execution/mod.rs`, and `crates/runtime/src/service/execution/context.rs`
- [ ] T007 [P] Make all newly spawned child agents write as `IndependentSession` in `crates/runtime/src/service/execution/root.rs`, `crates/runtime/src/service/execution/subagent.rs`, and `crates/runtime-execution/src/policy.rs`
- [ ] T008 Audit runtime robustness for locks, wake queues, and spawned tasks in `crates/runtime-agent-control/src/lib.rs`, `crates/runtime/src/service/execution/mod.rs`, and `crates/runtime/src/service/execution/collaboration.rs`

**Checkpoint**: Foundation ready - user story implementation can now begin in parallel

---

## Phase 3: User Story 1 - 父 Agent 管理可复用的子 Agent (Priority: P1) 🎯 MVP

**Goal**: 让 child agent 在单轮完成后回到 `Idle`、可继续接收 `send`，并能被父级显式 `close`

**Independent Test**: 创建 child、完成一轮、再次 `send` 新指令、通过 `observe` 看见其回到 `Idle`，最后 `close` 后拒收任何新消息

### Tests for User Story 1 ⚠️

- [ ] T009 [P] [US1] Add lifecycle and subtree-close regression coverage in `crates/runtime-agent-control/src/lib.rs` and `crates/runtime/src/service/execution/tests.rs`
- [ ] T010 [P] [US1] Add reusable-child tool flow coverage in `crates/runtime-agent-tool/src/tests.rs`

### Implementation for User Story 1

- [ ] T011 [US1] Implement `Pending -> Running -> Idle -> Terminated` transitions and `last_turn_outcome` updates in `crates/runtime-agent-control/src/lib.rs` and `crates/runtime/src/service/execution/collaboration.rs`
- [ ] T012 [US1] Replace `spawnAgent`/`sendAgent`/`closeAgent` implementations with `spawn`/`send`/`close` in `crates/runtime-agent-tool/src/spawn_tool.rs`, `crates/runtime-agent-tool/src/send_tool.rs`, `crates/runtime-agent-tool/src/close_tool.rs`, `crates/runtime-agent-tool/src/executor.rs`, and `crates/runtime-agent-tool/src/lib.rs`
- [ ] T013 [US1] Wire root-owned child creation and reusable-child scheduling in `crates/runtime/src/service/execution/root.rs` and `crates/runtime/src/service/execution/subagent.rs`
- [ ] T014 [US1] Enforce subtree terminate, mailbox discard of unacked messages, running-turn cancellation, and post-close `send` rejection in `crates/runtime/src/service/execution/collaboration.rs` and `crates/runtime-agent-control/src/lib.rs`

**Checkpoint**: User Story 1 should now support reusable child agents and explicit subtree close as the MVP

---

## Phase 4: User Story 2 - 父子消息在异步和重启场景下保持可恢复 (Priority: P2)

**Goal**: 把 mailbox 改造成 durable、可 replay、严格遵守 snapshot-drain 边界的协作通道

**Independent Test**: 运行中的 agent 只消费 turn-start batch；`Started` 后、`Acked` 前重启会重放相同 `delivery_id`；child 发父会触发父 wake

### Tests for User Story 2 ⚠️

- [ ] T015 [P] [US2] Add durable mailbox replay and started-before-acked crash coverage in `crates/runtime-agent-control/src/lib.rs`, `crates/runtime/src/service/execution/tests.rs`, and `crates/server/src/tests/runtime_routes_tests.rs`
- [ ] T016 [P] [US2] Add mailbox batch-boundary and duplicate-delivery prompt coverage in `crates/runtime-agent-loop/src/agent_loop/tests/regression.rs`

### Implementation for User Story 2

- [ ] T017 [US2] Implement `AgentMailboxQueued`/`AgentMailboxBatchStarted`/`AgentMailboxBatchAcked`/`AgentMailboxDiscarded` append paths with sender lifecycle snapshot fields and live-cache ordering in `crates/runtime/src/service/execution/collaboration.rs` and `crates/runtime-session/src/session_state.rs`
- [ ] T018 [US2] Implement turn-start snapshot drain with `delivery_id` dedup, pending replay, and batch-ack sequencing in `crates/runtime/src/service/execution/mod.rs` and `crates/runtime-session/src/turn_runtime.rs`
- [ ] T019 [US2] Inject mailbox batch context and duplicate-`delivery_id` guidance in `crates/runtime-agent-loop/src/subagent.rs` and `crates/runtime-prompt/src/contributors/workflow_examples.rs`
- [ ] T020 [US2] Implement child-to-parent wake, pending wake cleanup, durable discard handling, and preserved UI/timeline notification flow in `crates/runtime-agent-control/src/lib.rs`, `crates/runtime/src/service/execution/mod.rs`, and `crates/runtime-agent-loop/src/subagent.rs`

**Checkpoint**: User Story 2 should now provide durable mailbox recovery, snapshot-drain semantics, and parent wake on child messages

---

## Phase 5: User Story 3 - 维护者获得简化且可观测的协作接口 (Priority: P3)

**Goal**: 完成四工具公开面切换，提供可靠 `observe` 快照，并删除旧协作公开入口

**Independent Test**: registry/schema/prompt 中只剩 `spawn/send/observe/close`；直接父能 `observe` 直接子，非直接父被拒绝

### Tests for User Story 3 ⚠️

- [ ] T021 [P] [US3] Add `observe` authorization and snapshot-field coverage in `crates/runtime-agent-tool/src/tests.rs`, `crates/runtime/src/service/execution/tests.rs`, and `crates/server/src/tests/session_contract_tests.rs`
- [ ] T022 [P] [US3] Add removed-surface regression coverage for old collaboration tool names in `crates/runtime-agent-tool/src/tests.rs` and `crates/server/src/tests/runtime_routes_tests.rs`

### Implementation for User Story 3

- [ ] T023 [US3] Implement `observe` snapshot aggregation in `crates/runtime/src/service/execution/collaboration.rs` and `crates/runtime/src/service/execution/status.rs`
- [ ] T024 [US3] Replace old wait/deliver/resume tool wiring, create the `observe` tool struct with `Tool` trait impl, parameters schema, and capability metadata, and register the four-tool surface in `crates/runtime-agent-tool/src/observe_tool.rs`, `crates/runtime-agent-tool/src/collaboration_executor.rs`, `crates/runtime-agent-tool/src/wait_tool.rs`, `crates/runtime-agent-tool/src/deliver_tool.rs`, `crates/runtime-agent-tool/src/resume_tool.rs`, and `crates/runtime/src/builtin_capabilities.rs`
- [ ] T025 [US3] Update server and frontend callers to the new `spawn`/`send`/`observe`/`close` contract in `crates/server/src/http/routes/agents.rs`, `crates/server/src/http/routes/sessions/mutation.rs`, `frontend/src/lib/api/sessions.ts`, and `frontend/src/hooks/useAgent.ts`
- [ ] T026 [US3] Rewrite tool descriptions, spawn guidance, and workflow examples to the four-tool mental model in `crates/runtime-agent-tool/src/spawn_tool.rs`, `crates/runtime-prompt/src/contributors/workflow_examples.rs`, and `crates/core/src/action.rs`

**Checkpoint**: All user stories should now be independently functional, and the public collaboration surface should be fully simplified and observable

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: 清理残留、同步文档、完成整体验证

- [ ] T027 [P] Synchronize final implementation outcomes and caller removals in `specs/008-agent-four-tools/findings.md`, `specs/008-agent-four-tools/design-collaboration-runtime.md`, and `specs/008-agent-four-tools/migration.md`
- [ ] T028 Remove leftover old-surface references and dead collaboration code in `crates/runtime-agent-tool/src/lib.rs`, `crates/runtime-agent-tool/src/tests.rs`, `crates/server/src/tests/session_contract_tests.rs`, and `frontend/src/lib/api/sessions.ts`
- [ ] T029 Run runtime robustness review for locks, wake queues, spawned tasks, and error propagation in `crates/runtime-agent-control/src/lib.rs`, `crates/runtime/src/service/execution/mod.rs`, and `crates/runtime/src/service/execution/collaboration.rs`
- [ ] T030 Run repository validation commands from `specs/008-agent-four-tools/quickstart.md`: `cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`, `cd frontend && npm run typecheck`, `rg -n "waitAgent|sendAgent|closeAgent|deliverToParent|resumeAgent" crates frontend -g '*.rs' -g '*.ts' -g '*.tsx'`, and verify sub-session notification channels remain functional after migration

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)**: No dependencies - can start immediately
- **Phase 2 (Foundational)**: Depends on Phase 1 - blocks all user stories
- **Phase 3 (US1)**: Depends on Phase 2 - delivers the MVP and proves the Idle lifecycle model
- **Phase 4 (US2)**: Depends on Phase 2 and benefits from US1 lifecycle primitives, but remains independently testable through durable mailbox scenarios
- **Phase 5 (US3)**: Depends on Phase 2 and should land after the runtime semantics from US1/US2 stabilize
- **Phase 6 (Polish)**: Depends on all desired user stories being complete

### User Story Dependencies

- **US1 (P1)**: No dependency on other stories; this is the MVP
- **US2 (P2)**: Depends on the shared foundational mailbox/event primitives, but can be validated independently from US3
- **US3 (P3)**: Depends on the foundational contracts and becomes safest after US1/US2 stabilize, because `observe` and public-surface cleanup rely on the new runtime semantics

### Within Each User Story

- Tests and regression coverage before story completion
- Contract/runtime changes before caller migration
- Root/control-tree semantics before parent/child routing assertions
- Mailbox event append and replay before prompt injection and server/frontend adoption
- Final old-surface cleanup only after new surface is in place

### Parallel Opportunities

- `T002` and `T003` can run in parallel during setup
- `T006` and `T007` can run in parallel after `T004`
- `T009` and `T010` can run in parallel for US1
- `T015` and `T016` can run in parallel for US2
- `T021` and `T022` can run in parallel for US3
- `T027` can be prepared while `T030` validation is being coordinated

---

## Recommended Implementation Order

1. Complete Phase 1 (Setup) to lock contracts and migration inventory
2. Complete Phase 2 (Foundational) to lock core contracts, root ownership, and durable mailbox primitives
3. Deliver US1 (Phase 3) to prove the persistent child-agent model — this is the first mergeable increment
4. Add US2 (Phase 4) to make async/restart recovery reliable
5. Add US3 (Phase 5) to finish public-surface simplification and observability
6. Finish with Phase 6 cleanup and repository validation

**MVP 范围**: User Story 1 only

---

## Notes

- [P] tasks touch different files or non-overlapping concerns after prerequisites are complete
- 每个用户故事都保留了独立验证口径，避免"全部做完才知道有没有跑通"
- 建议 MVP 范围是 **User Story 1**
- 这版任务允许做颠覆式优化，但优化边界已经被锁定在 spec/plan：统一 `IndependentSession`、root 进入控制树、删除旧工具面、不保留兼容 shim
