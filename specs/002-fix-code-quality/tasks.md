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

- [ ] T031 [P] [US3] Replace `.lock().unwrap()` with `with_lock_recovery()` in `crates/plugin/src/peer.rs:300`
- [ ] T032 [P] [US3] Replace `.lock().expect("auth token lock poisoned")` with `lock_anyhow()?` in `crates/server/src/http/auth.rs:99`
- [ ] T033 [P] [US3] Replace 9 instances of `.expect()` with `with_lock_recovery()` or `lock_anyhow()` in `crates/runtime-registry/src/router.rs`

### Phase 4.2: Fix Timeout and Channel Operations

- [ ] T034 [US3] Replace `.expect("waiter should finish before timeout")` with `match` handling in `crates/runtime-agent-control/src/lib.rs:608`

### Phase 4.3: Validation

- [ ] T035 [US3] Run `rg '\.unwrap\(\)|\.expect\(' --type rust --glob '!tests/' --glob '!benches/' crates/` and verify no output
- [ ] T036 [US3] Run `cargo test --workspace --exclude astrcode` and verify all tests pass

**Checkpoint**: User Story 3 is complete when production code has zero `.unwrap()`/`.expect()` in lock/array/channel operations

---

## Phase 5: User Story 4 - 修复异步任务泄漏和持锁 await (Priority: P1)

**Goal**: 所有 `tokio::spawn` 创建的任务都有句柄管理和取消机制，且不存在持锁 await 模式

**Independent Test**: 搜索 `tokio::spawn` 调用，每个都有 `JoinHandle` 被保存和管理；搜索 `.lock().await.*.await` 模式不存在

### Phase 5.1: Create Task Managers

- [ ] T037 [P] [US4] Create `crates/runtime/src/service/execution/task_manager.rs` with `ExecutionTaskManager` struct (active_turns: DashMap<String, JoinHandle<()>>)
- [ ] T038 [P] [US4] Create `crates/runtime/src/service/execution/subagent_task_manager.rs` with `SubagentTaskManager` struct (active_children, child_cancel_tokens)
- [ ] T039 [P] [US4] Create `crates/runtime/src/bootstrap/plugin_load_handle.rs` with `PluginLoadHandle` struct (task_handle, state, completed_notify)

### Phase 5.2: Fix Fire-and-Forget Spawns

- [ ] T040 [US4] Update `crates/runtime/src/bootstrap.rs:238` to save PluginLoadHandle and provide wait_completion() method
- [ ] T041 [US4] Update `crates/runtime/src/service/execution/mod.rs:197` to use ExecutionTaskManager.spawn_turn()
- [ ] T042 [US4] Update `crates/runtime/src/service/execution/root.rs:168` to use ExecutionTaskManager.spawn_turn()
- [ ] T043 [US4] Update `crates/runtime/src/service/execution/subagent.rs:128` to use SubagentTaskManager.spawn_child()
- [ ] T044 [US4] Update `crates/runtime/src/service/watch_manager.rs:28` to save config_watch_handle in Mutex<Option<JoinHandle<()>>>
- [ ] T045 [US4] Update `crates/runtime/src/service/watch_manager.rs:46` to save agent_watch_handle in Mutex<Option<JoinHandle<()>>>

### Phase 5.3: Add Shutdown Methods

- [ ] T046 [P] [US4] Implement `ExecutionTaskManager::shutdown()` to abort all active turns
- [ ] T047 [P] [US4] Implement `SubagentTaskManager::shutdown()` to cancel all children
- [ ] T048 [P] [US4] Implement `WatchManager::shutdown()` to abort watch handles
- [ ] T049 [US4] Wire shutdown methods into `RuntimeService::shutdown()`

### Phase 5.4: Validation

- [ ] T050 [US4] Run `rg 'tokio::spawn' --type rust --glob '!tests/' crates/ | rg -v 'JoinHandle|let.*='` and verify no output
- [ ] T051 [US4] Run `rg '\.lock\(\)\.await\..*\.await' --type rust crates/` and verify no output
- [ ] T052 [US4] Run `cargo test --workspace --exclude astrcode` and verify all tests pass

**Checkpoint**: User Story 4 is complete when all spawns have JoinHandle management and no lock-then-await patterns exist

---

## Phase 6: User Story 5 - 统一错误处理链路 (Priority: P2)

**Goal**: 各 crate 的错误类型与 `core::AstrError` 兼容，且错误转换不丢失上下文

**Independent Test**: 各 crate 的自定义错误类型实现 `Into<AstrError>` 或通过 `#[source]` 保留原始错误；`map_err(|_| ...)` 丢弃上下文的用法不再存在

### Phase 6.1: Extend AstrError

- [ ] T053 [US5] Add 6 new variants to `crates/core/src/error.rs`: Protocol, Storage, Plugin, Config, Registry, AgentLoop (each with `#[source] inner` field)
- [ ] T054 [US5] Add `LockPoisoned { name: &'static str }` variant to AstrError

### Phase 6.2: Implement Error Conversions

- [ ] T055 [P] [US5] Implement `From<ProtocolError> for AstrError` in `crates/core/src/error.rs`
- [ ] T056 [P] [US5] Implement `From<StorageError> for AstrError` in `crates/core/src/error.rs`
- [ ] T057 [P] [US5] Implement `From<PluginError> for AstrError` in `crates/core/src/error.rs`
- [ ] T058 [P] [US5] Implement `From<ConfigError> for AstrError` in `crates/core/src/error.rs`
- [ ] T059 [P] [US5] Implement `From<RegistryError> for AstrError` in `crates/core/src/error.rs`
- [ ] T060 [P] [US5] Implement `From<AgentLoopError> for AstrError` in `crates/core/src/error.rs`

### Phase 6.3: Fix map_err Usage

- [ ] T061 [US5] Fix `map_err(|_| ...)` in `crates/src-tauri/src/main.rs:424` to preserve original error
- [ ] T062 [US5] Search and fix all other `map_err(|_| ...)` instances: `rg 'map_err\(\|_\|' --type rust --glob '!tests/' crates/`

### Phase 6.4: Validation

- [ ] T063 [US5] Run `rg 'map_err\(\|_\|' --type rust --glob '!tests/' crates/` and verify no output
- [ ] T064 [US5] Add error conversion tests in `crates/core/src/error.rs` to verify source chain preservation
- [ ] T065 [US5] Run `cargo test --workspace --exclude astrcode` and verify all tests pass

**Checkpoint**: User Story 5 is complete when all errors convert to AstrError with preserved context

---

## Phase 7: User Story 6 - 修正日志级别和消除静默错误 (Priority: P2)

**Goal**: 关键操作有正确的日志级别，且错误不被静默吞掉

**Independent Test**: 搜索关键错误不再使用 `debug!`；搜索 `.ok()` 和 `let _ =` 在非测试代码中的使用都有注释说明

### Phase 7.1: Fix Log Levels

- [ ] T066 [P] [US6] Change turn failed log from `warn!` to `error!` in `crates/runtime-agent-loop/src/hook_runtime.rs`
- [ ] T067 [P] [US6] Change hook call failed log from `debug!` to `error!` in `crates/runtime-agent-loop/src/hook_runtime.rs`
- [ ] T068 [P] [US6] Change critical operation logs from `debug!` to `error!` in `crates/core/src/runtime/coordinator.rs`

### Phase 7.2: Remove println/eprintln

- [ ] T069 [US6] Replace `println!` with `log::warn!` in `crates/runtime-config/src/loader.rs:92`
- [ ] T070 [US6] Search and replace all other `println!`/`eprintln!` in production code: `rg 'println!|eprintln!' --type rust --glob '!tests/' --glob '!examples/' crates/`

### Phase 7.3: Fix Silent Errors

- [ ] T071 [US6] Fix `.ok()` ignoring file operation error in `crates/storage/src/session/event_log.rs:254` (log error or return Result)
- [ ] T072 [US6] Audit all `.ok()` and `let _ =` usage: `rg '\.ok\(\)|let _ =' --type rust --glob '!tests/' crates/` and add comments explaining why errors are ignored

### Phase 7.4: Validation

- [ ] T073 [US6] Run `rg 'println!|eprintln!' --type rust --glob '!tests/' --glob '!examples/' crates/` and verify no output
- [ ] T074 [US6] Run `rg 'turn.*failed.*debug!|hook.*call.*failed.*debug!' --type rust crates/` and verify no output
- [ ] T075 [US6] Run `cargo test --workspace --exclude astrcode` and verify all tests pass

**Checkpoint**: User Story 6 is complete when critical operations use correct log levels and no silent errors exist

---

## Phase 8: User Story 7 - 拆分过大的 service 模块 (Priority: P2)

**Goal**: `runtime/src/service/` 下单文件不超过 800 行

**Independent Test**: `wc -l` 统计 service 目录下所有 `.rs` 文件，均不超过 800 行

**Note**: This phase should only start after US1-US4 are complete and stable

### Phase 8.1: Split service/mod.rs

- [ ] T076 [US7] Create `crates/runtime/src/service/session/create.rs` and move session creation logic from `service/mod.rs`
- [ ] T077 [US7] Create `crates/runtime/src/service/session/load.rs` and move session loading logic from `service/mod.rs`
- [ ] T078 [US7] Create `crates/runtime/src/service/session/delete.rs` and move session deletion logic from `service/mod.rs`
- [ ] T079 [US7] Create `crates/runtime/src/service/session/catalog.rs` and move session catalog logic from `service/mod.rs`
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

- [ ] T090 [US7] Run `find crates/runtime/src/service -name '*.rs' -exec wc -l {} \; | awk '$1 > 800 {print "FAIL: " $2 " has " $1 " lines"; exit 1}'` and verify no output
- [ ] T091 [US7] Run `cargo check --workspace` and verify no circular dependencies
- [ ] T092 [US7] Run `cargo test --workspace --exclude astrcode` and verify all tests pass (external API behavior unchanged)

**Checkpoint**: User Story 7 is complete when all service files are ≤800 lines and tests pass

---

## Phase 9: User Story 8 - 消除硬编码常量并统一依赖版本 (Priority: P3)

**Goal**: 所有硬编码的端口号、大小限制等值提取为常量，且所有依赖版本统一到 workspace

**Independent Test**: 搜索代码中的裸数字常量（62000、128000、20000、200000）不再存在；各 Cargo.toml 中的 toml、tracing、async-stream、tower 依赖使用 `workspace = true`

### Phase 9.1: Extract Constants

- [ ] T093 [P] [US8] Add port number constant (62000) to `crates/core/src/env.rs` as `pub const DEFAULT_SERVER_PORT: u16 = 62000;`
- [ ] T094 [P] [US8] Add size limit constants (128000, 20000, 200000) to `crates/runtime-config/src/constants.rs`
- [ ] T095 [US8] Search and replace all hardcoded constants: `rg '62000|128000|20000|200000' --type rust crates/` and replace with constant references

### Phase 9.2: Unify Workspace Dependencies

- [ ] T096 [P] [US8] Add `toml`, `tracing`, `async-stream`, `tower` to `Cargo.toml` workspace.dependencies section
- [ ] T097 [P] [US8] Update `crates/runtime-config/Cargo.toml` to use `toml = { workspace = true }`
- [ ] T098 [P] [US8] Update all crates using `tracing` to use `tracing = { workspace = true }`
- [ ] T099 [P] [US8] Update all crates using `async-stream` to use `async-stream = { workspace = true }`
- [ ] T100 [P] [US8] Update all crates using `tower` to use `tower = { workspace = true }`

### Phase 9.3: Validation

- [ ] T101 [US8] Run `rg '62000|128000|20000|200000' --type rust crates/` and verify only constant definitions remain
- [ ] T102 [US8] Run `rg 'toml.*=.*\{.*version|tracing.*=.*\{.*version|async-stream.*=.*\{.*version|tower.*=.*\{.*version' crates/*/Cargo.toml | rg -v 'workspace.*=.*true'` and verify only reasonable exceptions
- [ ] T103 [US8] Run `cargo check --workspace` and verify compilation passes
- [ ] T104 [US8] Run `cargo test --workspace --exclude astrcode` and verify all tests pass

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
