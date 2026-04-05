# Code Review — master (工作区全部更改)

## Summary
Files reviewed: 36 | New issues: 6 (0 critical, 0 high, 4 medium, 2 low) | Perspectives: 4/4
Build status: **compiles** (`cargo check --workspace` pass) | Tests: ~400+ passed, 0 failed

本次变更包含三大块：(1) compact 系统重构（hook 语义从覆盖改为追加、文件恢复机制集成、prompt 模板升级）；(2) 依赖升级（axum 0.8、reqwest 0.13、thiserror 2、notify 8 等）；(3) 前端协议切换到事件重放。

---

## 🔒 Security
*No security issues found.*

- 认证层完整，新端点继承 `require_auth`
- 插件 JSON 强类型反序列化，`overrideSystemPrompt` 向后兼容通过 `.or()` 映射，无注入风险
- `FileAccessTracker` 路径来自内部工具 metadata（已验证工作区边界），不接触外部输入
- 前端 SSE 数据经 `normalizeAgentEvent()` 类型校验，无 `innerHTML`/`eval` 汇点

---

## 📝 Code Quality

| Sev | Issue | File:Line | Consequence |
|-----|-------|-----------|-------------|
| Medium | `replace_capabilities_with_prompt_inputs` 仍 `pub` 且清空 hooks，存在静默回归风险 | `service/mod.rs` ~L214 | 外部调用者（测试/SDK）可能误用旧方法，导致插件 hooks 被静默丢弃。方法有 TODO 但无 `#[deprecated]` |
| Medium | `ToolCallResultDto.truncated` 字段后端有、前端丢弃 | `event.rs`, `types.ts`, `agentEvent.ts` | 用户可能看到不完整的工具输出但无任何截断提示 |

---

## ✅ Tests

**Run results**: ~400+ passed, 0 failed

| Sev | Untested scenario | Location |
|-----|-------------------|----------|
| Medium | `render_compact_system_prompt` 传入 `Some("   ")` 时是否跳过上下文追加 | `compaction.rs:159-160` |
| Medium | `/sessions/{id}/history` 缺少 404 路径测试 | `runtime_routes_tests.rs` |
| Low | `additionalSystemPrompt` 和 `overrideSystemPrompt` 同时存在的优先级未测试 | `plugin_hook_adapter.rs:337-339` |

---

## 🏗️ Architecture

| Sev | Inconsistency | Files |
|-----|--------------|-------|
| Medium | `runtime-agent-profiles` crate 有 Cargo.toml 但无 src/ 目录，且未加入 workspace members | `Cargo.toml`, `crates/runtime-agent-profiles/` |

---

## 🚨 Must Fix Before Merge
*(Critical/High only. If empty, diff is clear to merge.)*

**无阻塞项。** 编译通过，所有测试通过，无安全漏洞。

---

## 📎 建议修复（不阻塞合并）

1. **[ARCH-001]** 删除或完善 `runtime-agent-profiles` crate — 加入 workspace 并创建 `src/lib.rs`，或如果是误提交则删除
2. **[QUAL-001]** 给 `replace_capabilities_with_prompt_inputs` 标 `#[deprecated]` 或删除，引导调用者使用带 hooks 的版本
3. **[QUAL-002]** 前端消费 `truncated` 字段，在 UI 层展示截断提示
4. **[TEST-001]** 给 `render_compact_system_prompt` 补 whitespace 过滤测试
5. **[TEST-002]** 给 history endpoint 补 404 路径测试
6. **[TEST-003]** 补 `additionalSystemPrompt` + `overrideSystemPrompt` 同时存在的优先级测试

---

## 📎 Pre-Existing Issues (not blocking)
- `routes/sessions.rs` 和 `routes/mod.rs` 文档注释仍用 `:id`，实际路由已改为 `{id}`

## 🤔 Low-Confidence Observations
- `SessionHistorySnapshot` 已定义并导出但无外部消费者，可能是待实现的脚手架
- 集成测试用 `format!("{:?}", ...)` 做字符串断言，验证力度较弱，但对当前功能足够
- 前端 `loadSession` 的 phase 验证是硬编码拒绝列表，后端新增 `PhaseDto` 变体可能导致所有会话加载失败
