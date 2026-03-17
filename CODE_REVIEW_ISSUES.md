# Code Review — Prompt Orchestration System Overhaul

## Summary
Files reviewed: 14 (12 modified, 2 new) | New issues: 6 (0 critical, 2 high, 2 medium, 2 low) | Perspectives: 4/4

**Test results**: 116 passed, 0 failed, 0 skipped

---

## 🔒 Security
*No security issues found.*

---

## 📝 Code Quality

| Sev | Issue | File:Line | Consequence |
|-----|-------|-----------|-------------|
| **High** | `PromptPlan::merge()` 只处理 `extra_tools`，忽略了 `system_blocks`/`prepend_messages`/`append_messages` | `crates/agent/src/prompt/plan.rs:14` | 如果有外部调用方依赖此方法合并完整 contribution，会导致静默数据丢失 |
| **High** | `turn_runner.rs` 每个 step 都创建新的 `PromptComposer::with_defaults()`，contributor cache 无法跨 step 复用 | `crates/agent/src/agent_loop/turn_runner.rs:42` | 同一 turn 内多个 step 时，cache 永远是空的，`ContributorCacheHit` 诊断永远不会触发，缓存机制形同虚设 |
| Medium | `PromptContribution::merge()` 也只处理 `blocks`/`contributor_vars`/`extra_tools`，与旧版语义不一致 | `crates/agent/src/prompt/contribution.rs:15` | 与 `PromptPlan::merge()` 存在同样的语义断裂风险 |
| Low | `PromptBuildOutput.diagnostics` 在 `turn_runner.rs` 中被丢弃，调用方从未读取 | `crates/agent/src/prompt/composer.rs:57` | 诊断信息无法传递给上层，调试时无法利用 |

---

## ✅ Tests
**Run results**: 116 passed, 0 failed, 0 skipped

| Sev | Untested scenario | Location |
|-----|------------------|----------|
| Low | `PromptPlan::merge()` 的新语义（只合并 extra_tools）没有对应的测试覆盖 | `crates/agent/src/prompt/plan.rs:14` |
| Low | `PromptComposer` contributor cache TTL 过期路径没有测试 | `crates/agent/src/prompt/composer.rs:120` |

---

## 🏗️ Architecture

| Sev | Inconsistency | Files |
|-----|--------------|-------|
| Medium | `PromptPlan::merge()` 和 `PromptContribution::merge()` 是重构残留的 dead code——composer 已经在内部直接处理合并逻辑，但这两个方法保留了不完整的旧语义 | `plan.rs`, `contribution.rs` |

---

## ⚠️ 编译警告汇总（16 warnings）

以下为 dead code 警告，建议清理或添加 `#[allow(dead_code)]` 注释说明保留意图：

| 类型 | 符号 | 文件 |
|------|------|------|
| 未使用的 enum variant | `BlockKind::SystemPrompt` | `block.rs:9` |
| 未使用的 enum variant | `RenderTarget::{AppendUser, AppendAssistant}` | `block.rs:36-37` |
| 未使用的 enum variant | `ValidationPolicy::{Skip, Strict}` | `block.rs:43-44` |
| 未使用的 enum variant | `BlockCondition::{StepEquals, HasTool, VarEquals}` | `block.rs:50-53` |
| 未使用的 enum variant | `DiagnosticLevel::Error` | `diagnostics.rs:7` |
| 未使用的方法 | `BlockSpec::message_template` | `block.rs:137` |
| 未使用的方法 | `BlockSpec::with_validation_policy` | `block.rs:165` |
| 未使用的方法 | `BlockSpec::with_var` | `block.rs:185` |
| 未使用的方法 | `PromptPlan::merge` | `plan.rs:14` |
| 未使用的方法 | `PromptContribution::merge` | `contribution.rs:15` |
| 未使用的 re-export | `BlockMetadata`, `PromptBuildOutput`, `PromptComposerOptions`, `ValidationLevel`, `DiagnosticLevel/Reason/PromptDiagnostic/PromptDiagnostics`, `PromptTemplate` | `mod.rs` |
| 未使用的方法 | `AgentLoop::with_max_steps` | `agent_loop.rs:28` |
| 未使用的常量 | `DEFAULT_MAX_TOKENS` | `llm/anthropic.rs:19` |
| 未使用的方法 | `AnthropicProvider::new` | `llm/anthropic.rs:41` |
| 未使用的方法 | `TestEnvGuard::set_current_dir` | `test_support.rs:61` |

---

## 💡 建议

### 1. 修复 `PromptPlan::merge()`（高优先级）
当前实现只合并 `extra_tools`，应恢复完整语义或标记为 `#[deprecated]` 并移除调用方：
```rust
pub fn merge(&mut self, contribution: PromptContribution) {
    // 当前只做了这个：
    append_unique_tools(&mut self.extra_tools, contribution.extra_tools);
    // 缺失：system_blocks, prepend_messages, append_messages 的合并
}
```

### 2. 将 `PromptComposer` 提升到 turn 级别（高优先级）
在 `turn_runner.rs` 中，`PromptComposer` 应在 turn 开始时创建一次，然后在各 step 间复用：
```rust
// 当前（每个 step 都新建，cache 无效）：
let build_output = PromptComposer::with_defaults().build(&ctx).await;

// 建议（在 turn 开头创建，跨 step 复用 cache）：
let composer = PromptComposer::with_defaults();
// ... loop { composer.build(&ctx).await ... }
```

### 3. 清理 dead code
- 如果 `BlockKind::SystemPrompt`、`AppendUser/AppendAssistant` 等是为未来扩展预留的，添加 `#[allow(dead_code)]` 并注释说明
- 如果 `PromptPlan::merge()` 和 `PromptContribution::merge()` 不再需要，直接删除
- `mod.rs` 中的 re-export 如果仅在测试中使用，考虑移到 `#[cfg(test)]` 模块

### 4. 传递 diagnostics
考虑让 `turn_runner` 将 `build_output.diagnostics` 通过事件流传递给前端，方便调试 prompt 构建问题。

---

## ✅ 正面评价

- **模板系统设计良好**：`PromptTemplate` 简洁实用，`{{variable}}` 语法清晰，变量解析优先级（block → contributor → context → builtin）合理
- **依赖解析健壮**：`resolve_candidates` 正确处理了条件跳过、缺失依赖、循环依赖等边界情况
- **测试覆盖充分**：新增的 6 个测试覆盖了缓存、模板解析、条件跳过、严格验证等关键路径
- **诊断系统设计完整**：`PromptDiagnostics` 提供了结构化的调试信息，便于排查 prompt 构建问题
- **整体架构方向正确**：从简单的 block 列表升级为带条件、依赖、优先级的声明式 prompt 编排系统，为未来的 skill/plugin 系统打下了好基础
