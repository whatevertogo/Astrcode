# Code Review — dev (staged changes)

## Verification Status (2026-04-15)

### Resolved in code

- `conversation v1` 已成为唯一公开 product read surface，`/api/v1/terminal/*` 路由和对应 server tests 已删除。
- `TerminalControlFacts.root_status` 死数据路径已移除，不再执行无用 kernel 查询。
- `TerminalBlockPatchDto` 已补齐 `ReplaceMarkdown`，当最终 markdown 与流式前缀不一致时会发 corrective patch。
- `ensure_full_markdown_block` 的非前缀分支已有直接测试覆盖。
- `classify_transcript_error` 与 `cursor_is_after_head` 已补直接单元测试。
- application / server 间重复的 `truncate_summary` 已收敛为 `truncate_terminal_summary`。
- client SSE bridge 在没有任何消费者时会主动退出，不再继续无意义解析。

### Remaining / intentionally not changed

- `GET /__astrcode__/run-info` 暴露 bootstrap token 的问题仍然存在。这个问题要想真正修到位，需要一起重构 browser-dev bootstrap 发现机制，不能靠表面 header 校验伪修复。
- `current_timestamp_ms` 仍在 `client` 和 `cli` 各有一份实现。该逻辑非常小且语义稳定，本次未为此引入跨 crate 共享抽象。

## Summary
Files reviewed: 35 | New issues: 8 (0 critical, 1 high, 4 medium, 3 low) | Perspectives: 4/4

Branch: `dev` | Commit: `20a02185` (parent) | Scope: Terminal surface full vertical slice

---

## 🔒 Security

| Sev | Issue | File:Line | Attack path |
|-----|-------|-----------|-------------|
| Medium | Unauthenticated `GET /__astrcode__/run-info` exposes bootstrap token | `crates/server/src/bootstrap/mod.rs:118` | 任意本地进程 GET 该端点获取 token → exchange 为 API session → 完整 API 权限 |
| Low | CLI `--server-origin` 参数可触发 SSRF | `crates/cli/src/launcher/mod.rs:310-316` | `--server-origin http://169.254.169.254/...` 向任意 URL 发起请求 |
| Low | CLI `--server-binary` 可执行任意二进制 | `crates/cli/src/launcher/mod.rs:338-349` | `--server-binary /usr/bin/malicious` 执行任意程序（但需要本地 shell） |

**SEC-001 详情**：`serve_run_info` handler 无认证即返回 bootstrap token。服务端绑定 `127.0.0.1`，攻击面限于本地进程（恶意软件、被入侵的 VS Code 扩展等）。Token 有效期 24 小时，泄露后可获取完整 API 权限。建议加 Host 头校验或改为 Unix domain socket。

**SEC-002/003 详情**：CLI 参数来自本地用户，攻击者已有 shell 访问权限，实际风险极低。如需加固，可校验 origin 必须为 `127.0.0.1` 或 `localhost`。

**正面评价**：
- Token 比较使用恒定时间算法（`secure_token_eq`，XOR 累加），防时序攻击
- 所有 terminal 路由均通过 `require_terminal_auth` 校验
- Session path ID 使用字符白名单校验，防路径遍历
- 无硬编码密钥

---

## 📝 Code Quality

| Sev | Issue | File:Line | Consequence |
|-----|-------|-----------|-------------|
| Medium | `ensure_full_markdown_block` 在内容不匹配时静默丢弃更新 | `crates/server/src/http/terminal_projection.rs:460-471` | 客户端显示过时的 stream 文本，服务端 snapshot 显示正确内容，两者不一致 |
| Low | SSE bridge task 在 consumer drop 后继续解析 | `crates/client/src/lib.rs:253-290` | 不影响正确性，但产生不必要的 CPU/内存开销 |

**CQ-001 详情**：当 `AssistantMessage` 最终内容与 `ThinkingDelta`/`ModelDelta` 流式拼接结果不一致时（`content.starts_with(&existing)` 为 false），函数更新了服务端 block 状态但不发出任何 delta。`TerminalBlockPatchDto` 缺少 `ReplaceMarkdown` 变体，协议层无法通知客户端替换内容。

修复方案：在 `TerminalBlockPatchDto` 中新增 `ReplaceMarkdown { markdown: String }` 变体，当 `!starts_with` 时发出全量替换 delta。

**已验证无问题的区域**：block_index 幂等性、事件排序、UTF-8 安全性、cursor 校验、SSE 解析正确性、RwLock 使用模式、async 闭包安全性。

---

## ✅ Tests

**Run results**: 未执行（测试代理遇速率限制，以下为静态分析结果）

| Sev | Untested scenario | Location |
|-----|------------------|----------|
| Medium | `ensure_full_markdown_block` 内容不匹配分支（`!starts_with`）无测试 | `crates/server/src/http/terminal_projection.rs:460-471` |
| Low | `classify_transcript_error` 所有匹配分支无直接单元测试 | `crates/server/src/http/terminal_projection.rs:862-873` |
| Low | Client `stream_terminal` 的 `Lagged` broadcast 错误路径未被测试覆盖 | `crates/client/src/lib.rs:67-69` |
| Low | `cursor_is_after_head` 边界情况（相同 cursor）无直接测试 | `crates/application/src/terminal_use_cases.rs:247-255` |

**TEST-001 详情**：`project_terminal_stream_replay_freezes_patch_and_completion_semantics` 测试中所有内容都是增量的（`starts_with` 恒为 true），未覆盖 provider 重新生成内容导致 `!starts_with` 的场景。这是 CQ-001 的测试缺口。

**TEST-002 详情**：`classify_transcript_error` 通过字符串匹配分类错误类型（`context window` -> `ContextWindowExceeded` 等），但无测试验证。间接通过 snapshot 测试覆盖了 `rate_limit` 分支，其余分支未覆盖。

**已有的优秀测试覆盖**：
- Protocol conformance tests：5 个 fixture 覆盖 snapshot、delta append/patch/rehydrate、error envelope
- Client tests：7 个测试覆盖 auth 缓存、stream events（delta/rehydrate/disconnect）、error normalization、token 过期
- Launcher tests：5 个测试覆盖 RunInfo 发现、server spawn、ready 握手失败、token 校验
- Projection tests：3 个大型集成测试验证完整的 block 映射、stream replay、control state
- Terminal use cases：5 个端到端集成测试

---

## 🏗️ Architecture

| Sev | Inconsistency | Files |
|-----|--------------|-------|
| High | `root_status` 字段在 application 层填充但从未投影到 DTO | `terminal/mod.rs:24`, `terminal_use_cases.rs:300-301`, `terminal_projection.rs:132-143` |
| Medium | `truncate_summary` 在两个 crate 中重复实现 | `terminal_use_cases.rs:390-400`, `terminal_projection.rs:850-860` |
| Medium | `current_timestamp_ms` 在两个 crate 中重复实现 | `client/src/lib.rs:384-391`, `cli/src/launcher/mod.rs:256-263` |

**ARCH-001 详情**：`TerminalControlFacts.root_status` 从 `kernel.query_root_status()` 获取，但 `project_control_state` 从未读取该字段。`TerminalControlStateDto` 无对应字段。这是死数据路径：每次请求执行了 kernel 查询但结果被丢弃，且字段声明误导维护者认为该信息会传递到客户端。

修复方案：要么将 `root_status` 投影到 `TerminalControlStateDto`（新增字段），要么从 `TerminalControlFacts` 中移除该字段并在需要时再添加。

**ARCH-002/003 详情**：两处代码完全相同。`truncate_summary` 的 MAX_SUMMARY_CHARS 常量如果在一处修改而另一处未同步更新，会导致不一致的截断行为。

**已验证无问题的跨层契约**：
- DTO <-> Facts 字段对齐完整
- Protocol v1 命名空间正确隔离
- Client re-exports 完整
- Crate 依赖边界无违规（R001-R005）
- `lib.rs` re-exports 覆盖所有新公开类型

---

## 🚨 Must Fix Before Merge

*(1 High issue)*

1. **[ARCH-001]** `root_status` 死数据路径 — `crates/application/src/terminal/mod.rs:24`
   - Impact: 每次 terminal 请求执行无用 kernel 查询；字段声明误导维护者
   - Fix: 投影到 DTO（新增字段）或从 Facts 中移除

---

## 📋 Recommended Fixes (Medium)

1. **[CQ-001]** `ensure_full_markdown_block` 内容不一致时静默丢弃 — `terminal_projection.rs:460-471`
   - Impact: 客户端可能显示过时的流式文本
   - Fix: 新增 `ReplaceMarkdown` delta 变体

2. **[SEC-001]** run-info 端点无认证暴露 bootstrap token — `bootstrap/mod.rs:118`
   - Impact: 本地恶意进程可获取完整 API 权限
   - Fix: 加 Host 头校验或改为 Unix socket

3. **[TEST-001]** 内容不匹配分支缺少测试 — `terminal_projection.rs:460-471`
   - Impact: CQ-001 的回归风险无测试保障
   - Fix: 新增测试覆盖 `!starts_with` 场景

4. **[ARCH-002]** `truncate_summary` 重复 — 两个文件
   - Fix: 抽取到 `application::terminal` 模块

---

## 🤔 Low-Confidence / Low-Priority Observations

- **[CQ-002]** SSE bridge task 在 consumer drop 后不退出（不影响正确性，微小性能浪费）
- **[ARCH-003]** `current_timestamp_ms` 重复（风险极低，函数简单）
- **[SEC-002/003]** CLI 参数 SSRF/任意执行（需要本地 shell，实际威胁极低）
- **[TEST-002/003/004]** 低优先级测试缺口（`classify_transcript_error` 分支、`Lagged` 路径、`cursor_is_after_head` 边界）

---

## ✨ Positive Highlights

- 协议层使用 `#[serde(tag = "kind")]` adjacently-tagged enum，自描述且扩展友好
- Fixture-based conformance tests 冻结 wire contract，专业做法
- `ClientTransport` / `LauncherBackend` trait 抽象使得完整 mock 测试成为可能
- `TerminalDeltaProjector` 的 seed + project 设计同时服务 snapshot 和 stream replay
- `project_child_terminal_delivery` 对各种 child turn outcome 的穷举映射完整
- Token 比较使用恒定时间算法，auth 校验覆盖所有路由
