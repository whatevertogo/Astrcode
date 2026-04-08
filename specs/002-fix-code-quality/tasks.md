# Tasks: 修复项目代码质量问题

**Input**: Design documents from `/specs/002-fix-code-quality/`  
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/, quickstart.md

**Organization**: Tasks are grouped by user story so each story remains independently implementable and testable.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependency on unfinished tasks)
- **[Story]**: Which user story this task belongs to (`[US1]`, `[US2]`, `[US3]`, etc.)
- Every task includes exact file paths in the description

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Establish shared support functions and validation baseline before quality fixes

- [x] T001 Create `crates/core/src/support.rs` with `with_lock_recovery()` and `lock_anyhow()` functions from research.md
- [x] T002 Export support functions in `crates/core/src/lib.rs` as `pub mod support;`
- [x] T003 Run baseline validation: `cargo check --workspace && cargo clippy --all-targets --all-features -- -D warnings` and document current failures

**Checkpoint**: Support infrastructure ready for all user stories to use

---

## Phase 2: User Story 1 - 修复编译阻断问题 (Priority: P1) 🎯 MVP

**Goal**: 项目能够通过 `cargo check --workspace` 和 `cargo clippy` 检查

**Independent Test**: 运行 `cargo check --workspace` 和 `cargo clippy --all-targets --all-features -- -D warnings` 全部通过

- [x] T004 [US1] Fix Pattern trait bound error in `crates/runtime/src/service/watch_ops.rs:332` (change `&Cow<str>` to `&str` or implement Pattern)
- [x] T005 [US1] Remove unused import in `crates/server/src/http/routes/sessions/filter.rs:3`
- [x] T006 [US1] Run `cargo check --workspace` and verify no compilation errors
- [x] T007 [US1] Run `cargo clippy --all-targets --all-features -- -D warnings` and verify no warnings

**Checkpoint**: User Story 1 is complete when `cargo check` and `cargo clippy` pass with zero errors and warnings

---

## Phase 3: User Story 2 - 解除 core 对 protocol 的违规依赖 (Priority: P1)

**Goal**: `core` 和 `protocol` 之间不存在任何直接依赖

**Independent Test**: 检查 `crates/core/Cargo.toml` 的 `[dependencies]` 不包含 `astrcode-protocol`；`cargo check --workspace` 通过

### Phase 3.1: Preparation

- [x] T008 [P] [US2] Create `crates/core/src/plugin/descriptor.rs` and copy all Descriptor types from `crates/protocol/src/plugin/descriptors.rs` (CapabilityDescriptor, CapabilityDescriptorBuilder, CapabilityKind, PeerDescriptor, PeerRole, HandlerDescriptor, TriggerDescriptor, FilterDescriptor, ProfileDescriptor, SkillDescriptor, SkillAssetDescriptor)
- [x] T009 [P] [US2] Create `crates/core/src/plugin/context.rs` and copy context types from `crates/protocol/src/plugin/descriptors.rs` (InvocationContext, CallerRef, WorkspaceRef, BudgetHint)
- [x] T010 [P] [US2] Create `crates/core/src/plugin/metadata.rs` and copy metadata types from `crates/protocol/src/plugin/descriptors.rs` (SideEffectLevel, StabilityLevel, PermissionHint, DescriptorBuildError)
- [x] T011 [US2] DONE
- [x] T012 [US2] DONE

### Phase 3.2: Migrate SDK and Plugin Layer

- [x] T013 [P] [US2] Update `crates/sdk/src/lib.rs` to import Plugin types from `astrcode_core::plugin` instead of `astrcode_protocol`
- [x] T014 [P] [US2] Update `crates/sdk/src/tool.rs` to import CapabilityDescriptor from core
- [x] T015 [P] [US2] Update `crates/sdk/src/hook.rs` to import CapabilityDescriptor from core
- [x] T016 [P] [US2] Update `crates/plugin/src/capability_router.rs` to import from core
- [x] T017 [P] [US2] Update `crates/plugin/src/invoker.rs` to import from core
- [x] T018 [US2] DONE

### Phase 3.3: Migrate Runtime Layer

- [x] T019 [P] [US2] Update `crates/runtime/src/runtime_surface_assembler.rs` to import from core
- [x] T020 [P] [US2] Update `crates/runtime-agent-loop/src/agent_loop.rs` to import from core
- [x] T021 [P] [US2] Update `crates/runtime-agent-loop/src/approval_service.rs` to import from core
- [x] T022 [P] [US2] Update `crates/runtime-agent-loop/src/context_pipeline.rs` to import from core
- [x] T023 [P] [US2] Update `crates/runtime-agent-loop/src/context_window/prune_pass.rs` to import from core
- [x] T024 [P] [US2] Update `crates/runtime-agent-loop/src/prompt_runtime.rs` to import from core

### Phase 3.4: Migrate Server Layer

- [x] T025 [P] [US2] Update `crates/server/src/http/mapper.rs` to import CapabilityDescriptor from core
- [x] T026 [P] [US2] Update `crates/server/src/tests/runtime_routes_tests.rs` to import from core

### Phase 3.5: Cleanup and Validation

- [x] T027 [US2] DONE
- [x] T028 [US2] DONE
- [x] T029 [US2] DONE
- [x] T030 [US2] DONE

**Checkpoint**: User Story 2 is complete when core does not depend on protocol and all tests pass

---

## Phase 4: User Story 3 - 消除生产代码中的 panic 路径 (Priority: P1)

**Goal**: 所有锁获取、数组索引、超时等待不使用 `.unwrap()` / `.expect()`

**Independent Test**: 搜索非测试代码中的 `.unwrap()` 和 `.expect()` 调用，在锁获取、数组索引、channel 操作中不再存在

### Phase 4.1: Fix Lock Acquisition

- [x] T031 DONE
- [x] T032 DONE
- [x] T033 DONE

### Phase 4.2: Fix Timeout and Channel Operations

- [x] T034 DONE

### Phase 4.3: Validation

- [x] T035 DONE
- [x] T036 DONE

**Checkpoint**: User Story 3 is complete when production code has zero `.unwrap()`/`.expect()` in lock/array/channel operations

---

## Phase 5: User Story 4 - 修复异步任务泄漏和持锁 await (Priority: P1)

**Goal**: 所有 `tokio::spawn` 创建的任务都有句柄管理和取消机制，且不存在持锁 await 模式

**Independent Test**: 搜索 `tokio::spawn` 调用，每个都有 `JoinHandle` 被保存和管理；搜索 `.lock().await.*.await` 模式不存在

### Phase 5.1: Create Task Managers

- [x] T037 DONE
- [x] T038 DONE
- [x] T039 DONE

### Phase 5.2: Fix Fire-and-Forget Spawns

- [x] T040 DONE
- [x] T041 DONE
- [x] T042 DONE
- [x] T043 DONE
- [x] T044 DONE
- [x] T045 DONE

### Phase 5.3: Add Shutdown Methods

- [x] T046 DONE
- [x] T047 DONE
- [x] T048 DONE
- [x] T049 DONE

### Phase 5.4: Validation

- [x] T050 DONE
- [x] T051 DONE
- [x] T052 DONE

**Checkpoint**: User Story 4 is complete when all spawns have JoinHandle management and no lock-then-await patterns exist

---

## Phase 6: User Story 5 - 统一错误处理链路 (Priority: P2)

**Goal**: 各 crate 的错误类型与 `core::AstrError` 兼容，且错误转换不丢失上下文

**Independent Test**: 各 crate 的自定义错误类型实现 `Into<AstrError>` 或通过 `#[source]` 保留原始错误；`map_err(|_| ...)` 丢弃上下文的用法不再存在

### Phase 6.1: Extend AstrError

- [x] T053 DONE
- [x] T054 DONE

### Phase 6.2: Implement Error Conversions

- [x] T055 DONE
- [x] T056 DONE
- [x] T057 DONE
- [x] T058 DONE
- [x] T059 DONE
- [x] T060 DONE

### Phase 6.3: Fix map_err Usage

- [x] T061 DONE
- [x] T062 DONE

### Phase 6.4: Validation

- [x] T063 DONE
- [x] T064 DONE
- [x] T065 DONE

**Checkpoint**: User Story 5 is complete when all errors convert to AstrError with preserved context

---

## Phase 7: User Story 6 - 修正日志级别和消除静默错误 (Priority: P2)

**Goal**: 关键操作有正确的日志级别，且错误不被静默吞掉

**Independent Test**: 搜索关键错误不再使用 `debug!`；搜索 `.ok()` 和 `let _ =` 在非测试代码中的使用都有注释说明

### Phase 7.1: Fix Log Levels

- [x] T066 DONE
- [x] T067 DONE
- [x] T068 DONE

### Phase 7.2: Remove println/eprintln

- [x] T069 DONE
- [x] T070 DONE

### Phase 7.3: Fix Silent Errors

- [x] T071 DONE
- [x] T072 DONE

### Phase 7.4: Validation

- [x] T073 DONE
- [x] T074 DONE
- [x] T075 DONE

**Checkpoint**: User Story 6 is complete when critical operations use correct log levels and no silent errors exist

---

## Phase 8: User Story 7 - 拆分过大的 service 模块 (Priority: P2)

**Goal**: `runtime/src/service/` 下单文件不超过 800 行

**Independent Test**: `wc -l` 统计 service 目录下所有 `.rs` 文件，均不超过 800 行

**Note**: This phase should only start after US1-US4 are complete and stable

### Phase 8.1: Split service/mod.rs

- [x] T076 DONE
- [x] T077 DONE
- [x] T078 DONE
- [x] T079 DONE
- [ ] T080 [US7] Create `crates/runtime/src/service/turn/submit.rs` and move turn submission logic from `service/mod.rs`
- [ ] T081 [US7] Create `crates/runtime/src/service/turn/interrupt.rs` and move turn interrupt logic from `service/mod.rs`
- [ ] T082 [US7] Create `crates/runtime/src/service/turn/replay.rs` and move history replay logic from `service/mod.rs`
- [ ] T083 [US7] Update `crates/runtime/src/service/mod.rs` to re-export all submodules and keep only facade logic (≤800 lines)

### Phase 8.2: Split execution/mod.rs

- [ ] T084 [US7] Create `crates/runtime/src/service/execution/root.rs` (if not exists) and move root execution logic from `execution/mod.rs`
- [ ] T085 [US7] Create `crates/runtime/src/service/execution/subagent.rs` (if not exists) and move subagent execution logic from `execution/mod.rs`
- [ ] T086 [US7] Create `crates/runtime/src/service/execution/status.rs` and move status query logic from `execution/mod.rs`
- [ ] T087 [US7] Create `crates/runtime/src/service/execution/cancel.rs` and move cancel control logic from `execution/mod.rs`
- [ ] T088 [US7] Create `crates/runtime/src/service/execution/context.rs` and move execution context logic from `execution/mod.rs`
- [ ] T089 [US7] Update `crates/runtime/src/service/execution/mod.rs` to re-export all submodules and keep only facade logic (≤800 lines)

### Phase 8.3: Validation

- [x] T090 DONE
- [x] T091 DONE
- [x] T092 DONE

**Checkpoint**: User Story 7 is complete when all service files are ≤800 lines and tests pass

---

## Phase 9: User Story 8 - 消除硬编码常量并统一依赖版本 (Priority: P3)

**Goal**: 所有硬编码的端口号、大小限制等值提取为常量，且所有依赖版本统一到 workspace

**Independent Test**: 搜索代码中的裸数字常量（62000、128000、20000、200000）不再存在；各 Cargo.toml 中的 toml、tracing、async-stream、tower 依赖使用 `workspace = true`

### Phase 9.1: Extract Constants

- [x] T093 DONE
- [x] T094 DONE
- [x] T095 DONE

### Phase 9.2: Unify Workspace Dependencies

- [x] T096 DONE
- [x] T097 DONE
- [x] T098 DONE
- [x] T099 DONE
- [x] T100 DONE

### Phase 9.3: Validation

- [x] T101 DONE
- [x] T102 DONE
- [x] T103 DONE
- [x] T104 DONE

**Checkpoint**: User Story 8 is complete when all constants are extracted and dependencies are unified

---

## Phase 10: Polish & Cross-Cutting Concerns

**Purpose**: Final validation and documentation sync

- [ ] T105 [P] Run complete validation script from `quickstart.md` (all US1-US8 validation commands)
- [ ] T106 [P] Run `cargo fmt --all -- --check` and verify formatting
- [ ] T107 [P] Run `cargo clippy --all-targets --all-features -- -D warnings` and verify no warnings
- [ ] T108 [P] Run `cargo test --workspace --exclude astrcode` and verify all tests pass
- [ ] T109 [P] Run frontend validation: `cd frontend && npm run typecheck`
- [ ] T110 Update `CODE_QUALITY_ISSUES.md` to mark all issues as resolved

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies; start immediately
- **User Story 1 (Phase 2)**: Depends on Setup; must complete first (blocks development)
- **User Story 2 (Phase 3)**: Depends on Setup; can start after US1
- **User Story 3 (Phase 4)**: Depends on Setup; can start after US1
- **User Story 4 (Phase 5)**: Depends on Setup; can start after US1
- **User Story 5 (Phase 6)**: Depends on US1-US4 (error types stable)
- **User Story 6 (Phase 7)**: Depends on US1-US4 (logging stable)
- **User Story 7 (Phase 8)**: Depends on US1-US4 complete and stable (avoid simultaneous refactors)
- **User Story 8 (Phase 9)**: Independent, can run in parallel with US5-US7
- **Polish (Phase 10)**: Depends on all desired user stories being complete

### User Story Dependencies

- **US1 (P1)**: No dependencies; MUST complete first (blocks all development)
- **US2 (P1)**: Depends on US1 (compilation must work)
- **US3 (P1)**: Depends on US1 (compilation must work)
- **US4 (P1)**: Depends on US1 (compilation must work)
- **US5 (P2)**: Depends on US1-US4 (error types stable after panic fixes)
- **US6 (P2)**: Depends on US1-US4 (logging stable after robustness fixes)
- **US7 (P2)**: Depends on US1-US4 complete and stable (two-phase strategy)
- **US8 (P3)**: Independent, can run anytime after US1

### Parallel Opportunities

**After US1 completes, US2/US3/US4 can run in parallel**:
- US2 (Plugin migration): Touches core, protocol, sdk, plugin, runtime, server
- US3 (Panic fixes): Touches plugin, server, runtime-registry, runtime-agent-control
- US4 (JoinHandle management): Touches runtime/bootstrap, runtime/service/execution, runtime/service/watch_manager

**US5/US6/US8 can run in parallel after US1-US4**:
- US5 (Error unification): Touches core/error.rs, src-tauri
- US6 (Logging): Touches runtime-agent-loop, core, storage, runtime-config
- US8 (Constants): Touches core/env.rs, runtime-config/constants.rs, root Cargo.toml

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup
2. Complete Phase 2: User Story 1 (fix compilation)
3. **STOP and VALIDATE** using `cargo check` and `cargo clippy`
4. Verify development can proceed before tackling other stories

### Incremental Delivery (P1 Stories)

1. Complete US1 (compilation)
2. Run US2/US3/US4 in parallel (different file sets)
3. Validate after each story completes
4. Merge P1 stories before starting P2

### Full Delivery (All Stories)

1. Complete US1-US4 (P1 stories)
2. Run US5/US6/US8 in parallel
3. Complete US7 last (module split after stability)
4. Final validation and polish

---

## Parallel Execution Examples

### After US1: Run US2/US3/US4 in Parallel

```bash
# Terminal 1: US2 (Plugin migration)
# T008-T030

# Terminal 2: US3 (Panic fixes)
# T031-T036

# Terminal 3: US4 (JoinHandle management)
# T037-T052
```

### After US1-US4: Run US5/US6/US8 in Parallel

```bash
# Terminal 1: US5 (Error unification)
# T053-T065

# Terminal 2: US6 (Logging)
# T066-T075

# Terminal 3: US8 (Constants)
# T093-T104
```

---

## Notes

- `[P]` tasks target disjoint files and can be parallelized safely
- Story labels map every implementation task back to a single acceptance target
- Each story remains independently testable at its checkpoint
- US7 (module split) intentionally delayed until US1-US4 are stable
- US8 (constants) can run anytime, independent of other stories
- Total tasks: 110 (Setup: 3, US1: 4, US2: 23, US3: 6, US4: 16, US5: 13, US6: 10, US7: 17, US8: 12, Polish: 6)
