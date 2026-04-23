## 0. 前置约束

- [x] 0.1 在 `session-runtime-state-turn-boundary` 与 `server-session-runtime-isolation` 两个 change 中声明实施顺序依赖：先完成 isolation 的 HTTP/test 收口，再删除 `SessionState` runtime proxy。
  验证：`rg -n "server-session-runtime-isolation|session-runtime-state-turn-boundary|实施顺序|顺序依赖" openspec/changes/session-runtime-state-turn-boundary openspec/changes/server-session-runtime-isolation`

## 1. 边界文档与 owner 收口

- [x] 1.1 更新 `PROJECT_ARCHITECTURE.md` 与 `crates/session-runtime/src/state/mod.rs`、`crates/session-runtime/src/turn/mod.rs`、`crates/session-runtime/src/query/mod.rs` 的模块注释，明确 `SessionState` / `SessionActor` / `TurnRuntimeState` / `turn watcher` 的所有权边界。
  验证：`rg -n "TurnRuntimeState|watcher|SessionState" PROJECT_ARCHITECTURE.md crates/session-runtime/src/state/mod.rs crates/session-runtime/src/turn crates/session-runtime/src/query`

## 2. turn runtime control 迁移

- [x] 2.1 新增 `crates/session-runtime/src/turn/runtime.rs`，迁移 `ActiveTurnState`、`TurnRuntimeState`、`CompactRuntimeState`、`ForcedTurnCompletion`、`PendingManualCompactRequest`，并迁移测试 `turn_runtime_state_keeps_running_cache_and_active_turn_in_sync`、`recovery_resets_turn_runtime_to_idle_without_active_turn`、`stale_complete_generation_does_not_clear_resubmitted_turn`、`interrupt_execution_if_running_is_noop_after_turn_already_completed`，保持 generation / running / compact 语义不变。
  验证：`cargo test -p astrcode-session-runtime turn_runtime_state --lib`

- [x] 2.2 调整 `crates/session-runtime/src/actor/mod.rs`、`crates/session-runtime/src/lib.rs`、`crates/session-runtime/src/state/mod.rs`，让 `SessionActor` 直接持有 `turn_runtime: TurnRuntimeState`，`SessionState` 删除 `turn_runtime` 字段与相关 proxy 方法。
  验证：`rg -n "turn_runtime: TurnRuntimeState|prepare_execution|complete_execution_state|interrupt_execution_if_running|cancel_active_turn|is_running\\(|active_turn_id_snapshot\\(|manual_compact_pending\\(|compacting\\(|set_compacting\\(|request_manual_compact\\(" crates/session-runtime/src/state`

## 3. 调用链改经 turn-owned runtime

- [x] 3.1 更新 `crates/session-runtime/src/turn/submit.rs`、`crates/session-runtime/src/turn/interrupt.rs` 以及相关 helper，改由 actor/turn runtime handle 推进 prepare / complete / cancel / interrupt / deferred compact。
  验证：`cargo test -p astrcode-session-runtime turn::submit --lib` 和 `cargo test -p astrcode-session-runtime turn::interrupt --lib`

- [x] 3.2 更新 `crates/session-runtime/src/turn/finalize.rs`，使 compacting 切换与 deferred compact 读取改经 actor 的 `TurnRuntimeState`。
  验证：`cargo test -p astrcode-session-runtime turn::submit --lib`

- [x] 3.3 更新 `crates/session-runtime/src/command/mod.rs`，使 `request_manual_compact()`、`set_compacting()` 与等价控制路径改经 actor 的 `TurnRuntimeState`。
  验证：`cargo test -p astrcode-session-runtime command --lib`

- [x] 3.4 更新 `crates/session-runtime/src/query/service.rs` 的 runtime snapshot 读取路径，使 `session_control_state()` 不再通过 `SessionState` 读取 `active_turn_id`、`manual_compact_pending` 与 `compacting`。
  验证：`cargo test -p astrcode-session-runtime query::service --lib`

- [x] 3.5 更新 `crates/session-runtime/src/lib.rs` 与 `crates/session-runtime/src/actor/mod.rs`，使 `list_running_sessions()` 与 `snapshot()` 改经 actor 的 `TurnRuntimeState`。
  验证：`cargo check -p astrcode-session-runtime -p astrcode-application -p astrcode-server`

- [x] 3.6 在 `server-session-runtime-isolation` 已经先行收口测试边界后，迁移 `crates/application/src/agent/test_support.rs` 与 `crates/server/src/tests/config_routes_tests.rs` 中直接依赖 `SessionState` runtime proxy 的测试调用，改走调用方本地 test support 或稳定 runtime 路径，不再直接调用 `prepare_execution()`、`complete_execution_state()`、`is_running()`；若两个 change 叠栈，则必须先落 isolation 中对应的测试迁移。
  验证：`rg -n "prepare_execution\\(|complete_execution_state\\(|is_running\\(" crates/application crates/server`

## 4. watcher 归位与 query 纯读化

- [x] 4.1 新增 `crates/session-runtime/src/turn/watcher.rs`（或等价 turn-owned 模块），迁移 `wait_for_turn_terminal_snapshot()`、`try_turn_terminal_snapshot()`、`try_turn_terminal_snapshot_from_recent()`、`turn_snapshot_is_terminal()`、`record_targets_turn()`、`turn_events()`，并迁移测试 `wait_for_turn_terminal_snapshot_wakes_on_broadcast_event`、`wait_for_turn_terminal_snapshot_replays_only_once_while_waiting`、`wait_for_turn_terminal_snapshot_projects_legacy_reason_history`。
  验证：`cargo test -p astrcode-session-runtime wait_for_turn_terminal_snapshot --lib` 和 `cargo test -p astrcode-session-runtime turn_snapshot_is_terminal --lib`

- [x] 4.2 清理 `crates/session-runtime/src/query/service.rs` 中的 watcher 逻辑与过期注释，保留 `split_records_at_cursor()` 的 conversation stream replay 归属，确保 `query` 只保留纯读 / replay / snapshot 语义。
  验证：`rg -n "wait_for_turn_terminal_snapshot" crates/session-runtime/src/query`

## 5. 清理与全量验证

- [x] 5.1 清理 `crates/session-runtime/src/state/mod.rs`、`crates/session-runtime/src/turn/mod.rs`、`crates/session-runtime/src/query/mod.rs` 的导出与模块注释，确保目录结构与文档一致。
  验证：`cargo fmt --all -- --check`

- [x] 5.2 运行 `session-runtime` 直接相关测试与架构检查，确认本次 owner 迁移没有破坏边界。
  验证：`cargo test -p astrcode-session-runtime --lib`、`cargo check -p astrcode-session-runtime -p astrcode-application -p astrcode-server`、`node scripts/check-crate-boundaries.mjs`
