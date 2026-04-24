# Code Review — dev (未提交变更 + 暂存区大规模重构)

## Summary
- 审查范围：未提交变更（6 个 Rust 文件）+ 暂存区关键新模块抽样
- 未提交变更：7 个文件，+164 / -232 行
- 暂存区：407 文件，+33,468 / -45,009 行（application→server 重构、session-runtime→agent-runtime、新 host-session/plugin-host crate）
- 新问题：3（1 high, 1 medium, 1 low）
- 测试结果：24 passed, 0 failed（agent-runtime 16, core 2, adapter-llm 3, server 3）
- 编译检查：通过（workspace cargo check 无 warning）
- 视角：4/4

---

## Security

无安全新问题。

- 所有 HTTP 路由（mutation/query/stream/conversation）均调用 `require_auth`
- `validate_session_path_id` 白名单校验（`[a-zA-Z0-9\-_T]`），有效阻止路径注入
- `delete_project` 使用 `fs::canonicalize` 规范化路径
- `copy_dir_recursive` 跳过 symlink，防止符号链接穿越
- `normalize_prompt_request_text` 正确验证 skill invocation 一致性

---

## Code Quality

| Sev | Issue | File:Line | Consequence |
|-----|-------|-----------|-------------|
| High | `max_consecutive_failures` 错误用于 output continuation 限制 | session_runtime_port_adapter.rs:255（已修复） | output continuation 次数受失败重试上限控制，语义混淆。此 bug 已在未提交变更中修复。 |
| Medium | `copy_dir_recursive` 无递归深度限制 | mutation.rs:349 | 恶意或损坏的深层目录树可能导致栈溢出（桌面应用风险极低） |

### 已修复问题确认

`session_runtime_port_adapter.rs:255` 的修复是正确的——新增 `max_output_continuation_attempts` 配置项，在 `core/config.rs` 中独立声明（默认 3），带有 `.max(1)` 下限、serde skip_serializing_if、Debug 展示、validation 注册、以及 resolver 测试覆盖。修复将 `with_max_output_continuations(runtime.max_consecutive_failures)` 改为 `with_max_output_continuations(runtime.max_output_continuation_attempts)`。

---

## Tests

**Run results**: 24 passed, 0 failed, 0 skipped

| Test Suite | Result |
|---|---|
| agent-runtime::loop::tests (16) | OK |
| core::config::tests (2) | OK |
| adapter-llm::openai::dto::tests (3) | OK |
| server::mode::compiler::tests (3) | OK |

| Sev | Issue | Location |
|-----|-------|----------|
| Low | `copy_dir_recursive` 无单元测试 | mutation.rs:349 |

新增测试覆盖：
- `repeated_max_tokens_stops_at_configured_continuation_limit` — 直接验证 continuation 限制生效，与 bug 修复对应
- `child_mode_compile_uses_child_fork_mode_for_child_execution_fallback` — 验证 child fork mode 降级逻辑
- `assistant_message_*` / `user_and_tool_messages_*` (dto) — 验证 OpenAI 消息序列化边界

---

## Architecture

| Sev | Inconsistency | Files |
|-----|--------------|-------|
| — | 无新架构不一致 | — |

暂存区核心变更审查：
- **application → server 迁移**：`ApplicationError` → `ServerRouteError` 映射完整，conversation routes 正确切换
- **新 crate 边界**：`agent-runtime`（纯 runtime loop）、`host-session`（状态机 + mutation）、`plugin-host`（插件生命周期）职责清晰
- **AgentControlRegistry**：`spawn_with_storage` 正确校验 depth/concurrent 限制，`prune_finalized_agents_locked` 防止内存泄漏
- **host-session ports**：`EventStore` trait 提供 `recover_session` 默认实现（全量 replay），`SessionRecoveryCheckpoint` 结构合理
- **Crate 边界需验证**：`cargo check` 通过，建议合并前跑 `node scripts/check-crate-boundaries.mjs --strict`

---

## Must Fix Before Merge

*(无 Critical/High 级别阻断项——High 级 bug 已在未提交变更中修复。)*

确认修复已提交即可。

---

## Pre-Existing Issues (not blocking)

- `host-session/src/state.rs` 测试中使用 `unwrap()`（仅限 `#[cfg(test)]`，可接受）
- `plugin-host` reload 逻辑中 `commit_candidate().ok_or_else(...)` 的错误信息 `"candidate commit unexpectedly failed"` 缺少上下文，可考虑补充 snapshot_id

---

## Low-Confidence Observations

- `create_session(request.working_dir)` 未对 `working_dir` 做 `canonicalize`，与 `delete_project` 行为不一致。桌面应用中风险极低，但建议统一处理方式。
- `loop.rs` 中 `TurnExecutionContext` 有 15 个字段，构造复杂。当前通过 `new()` 封装，暂可接受，但若继续增长建议引入 builder。
