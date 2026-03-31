# Code Review — eba8720..HEAD

**审查范围**: 61 files, +2488 / -241 lines
**审查日期**: 2026-03-31
**验证结果**: cargo test 46/46 通过 | cargo fmt 通过 | tsc --noEmit 通过

---

## Findings

### 1. [Critical] CI 工作流分支名错误 — 永远不会触发

- **文件**: [.github/workflows/rust-check.yml:4](.github/workflows/rust-check.yml#L4), [.github/workflows/frontend-check.yml:5](.github/workflows/frontend-check.yml#L5), [.github/workflows/dependency-audit.yml](.github/workflows/dependency-audit.yml), [.github/workflows/tauri-build.yml](.github/workflows/tauri-build.yml)
- **问题**: 所有 4 个 CI 工作流配置 `branches: [main]`，但仓库默认分支是 `master`（`origin/HEAD -> origin/master`）。推送到 master 和以 master 为目标的 PR **永远不会触发 CI**。
- **修复**: 将所有 `branches: [main]` 改为 `branches: [master]`。

### 2. [High] CI 未排除 tauri crate — 会因 sidecar 缺失而失败

- **文件**: [.github/workflows/rust-check.yml:42](.github/workflows/rust-check.yml#L42)
- **问题**: `cargo test --workspace` 包含 tauri crate，CI 环境没有 sidecar 二进制文件，build script 会 panic（`PermissionDenied`）。
- **修复**: 改为 `cargo test --workspace --exclude astrcode`，或在 tauri build script 中对缺失 sidecar 做优雅降级。

### 3. [Medium] `last_storage_seq_from_path` 遍历全文件只为取最后一行

- **文件**: [crates/core/src/event/store.rs:327-335](crates/core/src/event/store.rs#L327-L335)
- **问题**: `EventLog::open()` 调用 `last_storage_seq_from_path()` 通过 `EventLogIterator` 遍历全文件来获取最后一个 seq。对于大型会话，每次打开已有会话都要读完全文件。
- **建议**: 使用 seek-from-end 或记录尾部 offset 的方式，仅读取文件末尾。

### 4. [Medium] E2E 测试断言薄弱 + 不稳定等待

- **文件**: [crates/server/src/e2e_tests.rs:262](crates/server/src/e2e_tests.rs#L262)
- **问题**: `e2e_session_replay_events` 用 `sleep(100ms)` 等待事件持久化，然后只断言 SSE 返回 200 和 `text/event-stream`，没有解析 SSE body 验证事件内容。在 CI 慢环境下 100ms 可能不够。
- **建议**: 使用 `wait_for_user_message_count` 类似的轮询模式，并解析 SSE body 确认包含预期事件。

### 5. [Medium] `shell.rs` UTF-8 截断可能在非 ASCII 边界 panic

- **文件**: [crates/tools/src/tools/shell.rs:159](crates/tools/src/tools/shell.rs#L159)
- **问题**: `output[..ctx.max_output_size().saturating_sub(truncation_msg.len())]` 按字节切片，如果截断点落在多字节 UTF-8 字符内部（如中文），`String[..n]` 会在非 char boundary 上 panic。当输出包含中文/Unicode 且总量接近 `max_output_size`（1MB）时必触发。
- **修复**: 使用 `output.char_indices()` 找到最近的 char boundary，或用 `floor_char_boundary()` (Rust 1.82+)。

### 6. [Low] E2E 测试中 path 规范化不一致

- **文件**: [crates/server/src/e2e_tests.rs:296](crates/server/src/e2e_tests.rs#L296) vs [crates/server/src/e2e_tests.rs:169](crates/server/src/e2e_tests.rs#L169)
- **问题**: 部分 E2E 测试对 `working_dir` 做了 `canonicalize()`，但 `e2e_multiple_sessions_isolation`（第 296 行）等处直接用 `.to_string_lossy()` 未做 canonicalize，与其他测试不一致。
- **建议**: 统一使用 `canonicalize()` 处理。

---

## Open Questions / Needs Discussion

1. **`ASTRCODE_HOME_DIR` 语义不一致风险**: `user_identity_md_path()` 在有 `ASTRCODE_HOME_DIR` 时拼接 `.astrcode/IDENTITY.md`，即把 `ASTRCODE_HOME_DIR` 当做 home 的父目录。如果 `ASTRCODE_HOME_DIR` 已经指向 `~/.astrcode`，路径会变成 `~/.astrcode/.astrcode/IDENTITY.md`。需确认与其他使用处（如 `session_path`）语义一致。

2. **`default_windows_shell()` 缓存缺失**: 每次调用 shell 工具都执行 `pwsh` 探测。高频场景下是不必要的开销。建议 `std::sync::OnceLock` 缓存。

3. **Doc tests 标记 `ignore`**: `EventLog::iter_from_path` 和 `EventLog::replay_to` 的 doc test 被忽略。建议改为 `no_run` 或修复为可运行。

---

## Security Notes

### [Medium/Pre-existing] `shell` 工具 `shell` 参数允许指定任意可执行文件

- **文件**: [crates/tools/src/tools/shell.rs:183-206](crates/tools/src/tools/shell.rs#L183-L206)
- **问题**: `ShellArgs.shell` 字段允许 LLM 指定任意程序作为"shell"。LLM 可传入 `/usr/bin/curl` 等非 shell 程序，绕过"shell 解释器"的预期。这是预存问题，本 diff 仅做了 accessor 重构。
- **修复方向**: 限制 `shell` 到固定白名单（`/bin/sh`, `/bin/bash`, `pwsh`, `powershell`），或完全移除该参数。

### [Medium/New] IDENTITY.md 无验证/长度限制 — 纵深防御缺失

- **文件**: [crates/runtime/src/prompt/contributors/identity.rs:42-60](crates/runtime/src/prompt/contributors/identity.rs#L42-L60)
- **问题**: 新增的 `IdentityContributor` 从 `~/.astrcode/IDENTITY.md` 读取内容并注入 LLM 系统提示，无长度限制和内容验证。如果本地其他进程（或被 LLM 通过 shell 工具）写入恶意指令，可改变 Agent 行为。
- **修复方向**: 添加最大长度检查（如 4096 字节），加载自定义 identity 时记录 info 日志。

---

## Pre-existing Issues (Not Introduced by This Diff)

- `AllowAllPolicyEngine` 作为默认策略允许一切，生产部署前需替换。
- `EventLog` 非 `Send`：内部持有 `BufWriter<File>`，当前架构正确但需注意。

---

## Verification Log

| 命令 | 结果 |
|------|------|
| `cargo fmt --all -- --check` | 通过 |
| `cargo test --workspace --exclude astrcode` | 46/46 通过, 2 doc tests ignored |
| `npm run typecheck` (frontend) | 通过 |

---

## 总体评估

代码质量整体良好。架构清晰，新增的 capability metadata、identity contributor、E2E 测试设计合理。**最紧迫的问题是 CI 工作流分支名错误（#1），导致 CI 完全不工作。** 修复 #1 和 #2 后 CI 即可正常运行。

## 建议优先级

1. **立即修复**: CI 工作流 `main` → `master`（#1），排除 tauri crate（#2）
2. **短期改进**: shell.rs UTF-8 截断防 panic（#5），E2E 测试断言增强（#4），path 规范化统一（#6）
3. **中期优化**: `last_storage_seq_from_path` 性能优化（#3），shell 探测缓存
