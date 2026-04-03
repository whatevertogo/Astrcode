# Code Review — 大规模代码清理 (54 files, -3097/+445)

## Summary
Files reviewed: 54 | New issues: 6 (0 critical, 2 high, 3 medium, 1 low) | Perspectives: 4/4

这是一次大规模的代码清理工作，主要删除冗余注释、合并测试、简化错误处理。
**⚠️ 发现 2 个高优先级测试覆盖问题需要确认。**

---

## 🔒 Security
**No security issues found.**

更改不涉及：
- 无新的输入sink（SQL/shell/template）
- 无硬编码密钥
- 无认证/授权绕过
- 无不安全的反序列化

---

## 📝 Code Quality

| Sev | Issue | File:Line | Consequence |
|-----|-------|-----------|-------------|
| High | 删除了大量测试文件和测试用例，可能影响回归测试能力 | 多个文件 | 功能回归风险增加 |

### 详细分析

#### 1. 测试删除问题 (High)

**删除的测试文件**：
- `crates/core/tests/app_home_integration.rs` (29行) - 完整删除
- `crates/server/src/browser_bootstrap_tests.rs` (270行) - 完整删除
- `crates/server/src/windows_subsystem_tests.rs` (62行) - 完整删除

**删除的测试用例**（部分列表）：
- `crates/core/src/action.rs`: 删除了3个 `model_content_*` 测试
- `crates/core/src/project.rs`: 删除了2个平台特定测试
- `crates/runtime-llm/src/anthropic.rs`: 合并了多个测试，净减少测试数
- `crates/storage/src/session/query.rs`: 删除了4个测试用例

**影响**：这些测试可能覆盖了一些边缘情况，删除后会降低回归测试能力。

#### 2. 注释删除 (Medium)

删除了大量文档注释，包括：
- `crates/runtime/src/service/turn_ops.rs` 中 `TurnExecutionStats` 的字段注释
- 多处的函数文档注释

**建议**：某些注释解释了"为什么"（如pending_prompt_tokens的设计原因），删除后可能影响后续维护者理解代码意图。

#### 3. 正面改进

- **错误处理简化**：`AstrError` 类型替换为独立函数 `io_error`、`parse_error`，更符合 Rust 惯例
- **代码简化**：`ToolCapabilityInvoker` 中的 `tool_ctx` 构建更清晰
- **API简化**：`check_cancel` 不再需要工具名称参数

---

## ✅ Tests

**Run results**: 测试通过（cargo test --workspace --exclude astrcode --lib）

| Sev | Issue | Location |
|-----|-------|----------|
| Medium | 删除的测试用例无替代覆盖 | 多个文件 |

### 测试覆盖率变化

**删除的测试类型**：
1. **集成测试** - `app_home_integration.rs`, `browser_bootstrap_tests.rs`
2. **边缘情况测试** - 如 `model_content_uses_real_newline_for_failed_tools`
3. **平台特定测试** - Windows/Unix 的 `project_dir_name` 测试

**保留的测试**：
- 核心功能测试仍然保留
- 合并后的测试（如 `response_to_output_parses_text_tool_use_and_thinking`）覆盖了多个场景

---

## 🏗️ Architecture

| Sev | Issue | Files |
|-----|-------|-------|
| Low | `check_cancel` API变更未在所有工具中统一 | crates/tools/ |

### 详细分析

1. **错误处理重构**：`AstrError` → `io_error`/`parse_error` 函数
   - 符合 Rust 惯例，无需构造器类型
   - 所有调用点已更新

2. **API简化**：`check_cancel(ctx.cancel())` 替代 `check_cancel(ctx.cancel(), "toolName")`
   - 部分工具已更新，部分未更新（不一致）

---

## 🚨 Must Fix Before Merge

### 1. [HIGH] CancelToken 和 ToolContext 测试被删除无替代

**文件**: `crates/core/tests/app_home_integration.rs` (完整删除)

**删除的关键测试**：
- `cancel_token_clone_observes_shared_cancellation` - 测试取消令牌的并发行为
- `tool_context_preserves_explicit_execution_roots` - 测试工具上下文构造

**影响**: `CancelToken` 是运行时取消传播的核心原语，删除测试可能导致回归风险。

### 2. [HIGH] 新增的 `resolve_relative_path` 函数无测试

**文件**: `crates/plugin/src/loader.rs:41-59`

**未测试场景**:
- `require_components_gt_1=true` 正确跳过裸可执行文件名
- `require_components_gt_1=false` 解析单组件相对路径
- 绝对路径返回不变
- `None` 值处理

**影响**: 插件发现逻辑的微妙分支条件，容易出错。

### 3. [MEDIUM] E2E API 端点测试被删除

**文件**: `crates/server/src/e2e_tests.rs`

**删除的测试**：
- `e2e_session_replay_events` - SSE 事件回放
- `e2e_session_interrupt` - 会话中断端点
- `e2e_auth_exchange_flow` - 认证令牌交换
- `e2e_model_list` - 模型列表端点

### 4. [MEDIUM] SDK 契约测试被删除

**文件**: `crates/sdk/src/tests.rs`

**删除的测试** (8个)：
- 工具注册、策略钩子、错误转换等 SDK 契约测试

**影响**: 降低了对插件作者的 SDK 向后兼容性保障

---

## 📎 Pre-Existing Issues (not blocking)

无

---

## 🤔 Low-Confidence Observations

1. **注释删除的边界**：某些删除的注释（如 `TurnExecutionStats` 字段注释）可能对后续维护者有价值，但代码本身足够清晰，删除也是合理的。

2. **测试合并策略**：将多个小测试合并为一个更大的测试（如 anthropic.rs）是常见做法，但可能降低测试失败时的诊断精度。

---

## 建议

1. **在提交前**：确认删除的测试确实冗余或已过时
2. **Commit message**：清晰说明本次清理的范围和原因
3. **后续**：考虑添加注释说明为什么某些设计是这样的（如 `pending_prompt_tokens` 的延迟计费机制）
