## 0. 前置约束

- [ ] 0.1 在 `server-session-runtime-isolation` 与 `session-runtime-state-turn-boundary` 两个 change 中声明实施顺序依赖：先完成本 change 的 HTTP/test 收口，再删除 `SessionState` runtime proxy。验证：`rg -n "server-session-runtime-isolation|session-runtime-state-turn-boundary|实施顺序|顺序依赖" openspec/changes/server-session-runtime-isolation openspec/changes/session-runtime-state-turn-boundary`

## 1. 校正 application 边界合同

- [x] 1.1 在 `crates/application/src/terminal/` 下定义完整的 application-owned terminal / conversation contracts，覆盖 8 种 block、3 种 delta、10 种 patch、4 种 status，以及 plan / system note / child handoff / transcript error 等叶子结构；同时替换 `TerminalFacts.transcript` 与 `TerminalStreamReplayFacts.replay` 中的 runtime 字段。验证：`cargo check -p astrcode-application`
- [x] 1.2 在 `crates/application/src/terminal/runtime_mapping.rs`（或等价结构）实现 runtime facts -> application contracts 的映射，并更新 `crates/application/src/terminal_queries/snapshot.rs` 让 `conversation_snapshot_facts()` / `conversation_stream_facts()` 返回 application-owned terminal surface。验证：`cargo test -p astrcode-application terminal_queries --lib`
- [x] 1.3 在 `crates/application/src/terminal/stream_projection.rs`（或等价结构）搬入 `ConversationStreamProjectorState` 的协调逻辑：状态 owner 迁到 `application`，内部仍可使用 runtime `ConversationStreamProjector`，server 只消费 application 暴露的 replay / durable / live / recover surface。验证：`cargo test -p astrcode-server conversation::tests --lib`
- [x] 1.4 在 `crates/application/src/session_use_cases.rs`、`crates/application/src/ports/app_session.rs`、`crates/application/src/test_support.rs` 中引入 application-owned fork selector，去掉 `server -> application` 边界上的 runtime `ForkPoint` 泄漏，并固定 `App::fork_session()` 对外只返回 `SessionMeta`。验证：`cargo check -p astrcode-application -p astrcode-server`
- [x] 1.5 把创建 session 的 working-dir 规范化/校验下沉到 `application` 用例或其 port 实现中，移除 route 层对 runtime `normalize_working_dir` 的依赖。验证：`cargo check -p astrcode-application -p astrcode-server`

## 2. 迁移 server route 与 mapper

- [x] 2.1 重写 `crates/server/src/http/terminal_projection.rs`，只映射 `application` 的 terminal contracts 到 protocol DTO，移除对 runtime block / delta / patch / status 枚举的直接匹配。验证：`rg -n "ConversationBlockFacts|ConversationDeltaFacts|ConversationBlockPatchFacts|ConversationBlockStatus|ToolCallBlockFacts|astrcode_session_runtime" crates/server/src/http/terminal_projection.rs`
- [x] 2.2 重写 `crates/server/src/http/routes/conversation.rs`，通过 `application` 的 stream surface 完成 replay / delta / rehydrate 路径，不再直接持有 runtime `ConversationStreamProjector`；同时更新 route-local tests 的测试数据构造，不再直接构造 runtime `ConversationStreamReplayFacts`。验证：`rg -n "ConversationStreamProjector|ConversationStreamReplayFacts as Runtime|astrcode_session_runtime" crates/server/src/http/routes/conversation.rs`
- [x] 2.3 重写 `crates/server/src/http/routes/sessions/mutation.rs`，改为消费 application-owned fork selector；route 层不再直接构造 runtime `ForkPoint`。验证：`rg -n "ForkPoint|astrcode_session_runtime" crates/server/src/http/routes/sessions/mutation.rs`
- [x] 2.4 重写 `crates/server/src/http/routes/sessions/mod.rs` 的 create-session 输入校验路径，确保 route 层不再直接调用 runtime `normalize_working_dir`。验证：`rg -n "normalize_working_dir|astrcode_session_runtime" crates/server/src/http/routes/sessions/mod.rs`

## 3. 收口 server 测试与内部依赖

- [x] 3.1 为 `crates/server/src/tests/` 增加语义化 route test harness（可放入 `test_support.rs` 或等价模块），封装“已完成 turn”“未完成 turn”“running session”等场景搭建，避免测试主体直接操作 `SessionState`。验证：`cargo test -p astrcode-server --lib`
- [x] 3.2 修改 `crates/server/src/tests/session_contract_tests.rs` 与 `config_routes_tests.rs`，移除测试主体中的 `get_session_state()`、`writer.append()`、`translate_store_and_cache()`、`prepare_execution()` 直连调用。验证：`rg -n "get_session_state\\(|translate_store_and_cache\\(|prepare_execution\\(|writer\\." crates/server/src/tests`
- [x] 3.3 收口 `server` 中对 `astrcode_session_runtime` 的直接使用，只允许保留在 bootstrap 与明确的内部 harness；复核 bootstrap、HTTP route、mapper、tests 的 import 分布。验证：`rg -n "astrcode_session_runtime" crates/server/src`

## 4. 全量验证与边界检查

- [x] 4.1 运行 application 与 server 的编译/测试验证，确保新 terminal contracts、stream projection、fork selector 与 route 迁移没有回归。验证：`cargo test -p astrcode-application --lib`、`cargo test -p astrcode-server --lib`
- [x] 4.2 验证 HTTP 层已经实现“零 runtime import”。验证：`rg "astrcode_session_runtime" crates/server/src/http`
- [x] 4.3 运行 crate 边界检查，并人工复核 `server` 是否仍然只在 bootstrap / 内部 harness 中直接接触 runtime。验证：`node scripts/check-crate-boundaries.mjs` 与 `rg -n "astrcode_session_runtime|ConversationStreamProjector|ConversationBlockFacts|ConversationBlockPatchFacts|ForkPoint|normalize_working_dir" crates/server/src`
