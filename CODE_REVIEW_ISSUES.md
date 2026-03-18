# Code Review — master (全量审查)

## Summary
Files reviewed: ~80 | New issues: 21 (1 critical, 6 high, 7 medium, 7 low) | Perspectives: 4/4

**Test results**: Rust 158 passed, 0 failed | Frontend 14 passed, 0 failed

---

## 🔒 Security

| Sev | Issue | File:Line | Attack path |
|-----|-------|-----------|-------------|
| **High** | `resolve_path` 不强制工作目录边界，绝对路径和 `../` 可读写任意文件 | `crates/tools/src/tools/fs_common.rs:20-28` | LLM 调用 `readFile("/etc/shadow")` 或通过 `../` 遍历，工具不会拒绝 |
| **High** | `findFiles` glob 模式可通过绝对路径覆盖 root 或 `../` 逃逸 | `crates/tools/src/tools/find_files.rs:59-66` | LLM 传入 `pattern="/etc/*.conf"` → `root.join("/etc/*.conf")` 变为 `/etc/*.conf` |
| **Medium** | symlink 绕过 `resolve_path` 沙箱（仅做词法规范化，不展开 symlink） | `crates/tools/src/tools/fs_common.rs:20-28` | `working_dir` 内 symlink 指向 `/etc/passwd`，工具可读写 |
| **Medium** | `shell` 工具允许 LLM 指定任意 shell 程序 | `crates/tools/src/tools/shell.rs:156-181` | LLM 指定 `"shell": "python3"` 将命令当 Python 代码执行 |
| Low | `run.json` 文件权限过于宽松（默认 644），同机用户可读 bootstrap token | `crates/server/src/main.rs:870-888` | 多用户服务器上其他用户读取 `~/.astrcode/run.json` 获取 token |
| Low | Token 比较使用 `==` 而非常量时间比较 | `crates/server/src/main.rs:622` | 理论上可通过计时侧信道泄露 bootstrap token（已通过 localhost-only 缓解） |
| Info | Vite dev server `/__astrcode__/run-info` 无认证暴露 token | `frontend/vite.config.ts:92-116` | localhost 进程可获取 token（开发环境专用，风险有限） |

---

## 📝 Code Quality

| Sev | Issue | File:Line | Consequence |
|-----|-------|-----------|-------------|
| **Medium** | `delete_project` 中 interrupt 是 fire-and-forget，remove 立即执行；运行中的 turn 持有 `Arc<SessionState>` 会在磁盘文件删除后继续写入 | `crates/agent/src/service/session_ops.rs:98-121` | 数据不一致：运行中的 turn 写入已删除的日志文件 |
| **Medium** | `append_and_broadcast_blocking` 嵌套 `block_in_place` + `block_on` + `spawn_blocking`，tokio 反模式 | `crates/agent/src/service/turn_ops.rs:139-147` | 高频事件流下可能导致 tokio worker 线程饥饿或死锁 |
| **Medium** | `block_in_place` 在 `tokio::spawn` 内调用，current-thread runtime 下会 panic | `crates/agent/src/service/turn_ops.rs:139-147` | Tauri 使用 current-thread runtime 时触发 panic |
| **Medium** | `running` 标志在 cancel token 安装前设置，存在竞态 | `crates/agent/src/service/turn_ops.rs:25-37` | 快速提交后立即中断，中断可能被丢弃，turn 不被取消 |
| **Medium** | `read_last_phase` 加载整个 JSONL 文件只为取最后一个 phase | `crates/agent/src/event_log/query.rs:321-328` | 长会话列表演变为 O(N×M)，造成延迟和内存压力 |
| **Medium** | `turnRouting.ts` 中 `shift()` 变异数组，StrictMode 双渲染时错误消费 | `frontend/src/lib/turnRouting.ts:16` | React StrictMode 下第二次调用消费不同的 session ID，turn 路由到错误的会话 |
| Low | OpenAI provider 静默吞掉 `arguments` JSON 解析错误，包装为 `Value::String` | `crates/agent/src/llm/openai.rs:246-248` | 调试困难：错误信息指向"参数格式错误"而非 JSON 解析失败 |
| Low | `default_windows_shell()` 每次调用都 spawn 子进程检测 PowerShell | `crates/tools/src/tools/shell.rs:184-196` | 每次 shell 工具调用额外 ~100-500ms 开销 |
| Low | `resolve_api_key` 启发式判断会把某些 API key 字面量误判为环境变量名 | `crates/agent/src/config.rs:134-137` | 特定格式的 key（如 `sk_live_ABC123_KEY`）会被错误地当作环境变量查找 |
| Low | Legacy 事件用行号作为 `storage_seq`，可能与真实值冲突 | `crates/agent/src/events.rs:93-110`, `store.rs:158` | SSE `last-event-id` 回放时可能跳过或重复事件 |
| Low | `read_last_timestamp` 遇到 Error 事件跳过，`updated_at` 可能过时 | `crates/agent/src/event_log/query.rs:284-307` | 会话列表中 `updated_at` 不反映最后的错误事件时间 |

---

## ✅ Tests

**Run results**: Rust 158 passed, 0 failed | Frontend 14 passed, 0 failed

| Sev | Untested scenario | Location |
|-----|------------------|----------|
| **High** | Shell 工具缺少取消、非零退出码、stderr 捕获、自定义 shell/cwd、spawn 失败等测试 | `crates/tools/src/tools/shell.rs` tests |
| **High** | `ToolRegistry::execute()` 未知工具名和工具执行失败的分支未测试 | `crates/agent/src/tool_registry.rs:59-82` |
| **High** | `normalize_session_id` / `normalize_working_dir` 无任何测试 | `crates/agent/src/service/session_ops.rs:180-221` |
| **Medium** | Server 核心路由（submit_prompt, interrupt_session, SSE events 等）完全没有 handler 测试 | `crates/server/src/main.rs` |
| **Medium** | `normalizeAgentEvent` 只测了 protocol gate，8 种事件类型的正常/异常解析路径均未覆盖 | `frontend/src/lib/agentEvent.test.ts` |
| **Medium** | `EventTranslator` 缺少 Error/Interrupted、ThinkingDelta、legacy turn ID、cursor 过滤测试 | `crates/agent/src/service/replay.rs` tests |
| **Medium** | `resolveSessionForTurn` 缺少 null turnId 和 null activeSessionId 的边界测试 | `frontend/src/lib/turnRouting.test.ts` |
| **Medium** | `normalize_session_id` 双前缀剥离（`"session-session-abc"`）无直接单测 | `crates/agent/src/service/session_ops.rs:180-186` |
| **Medium** | `validate_config` 6 个验证分支完全无测试 | `crates/agent/src/config.rs:315-388` |
| Medium | `normalize_working_dir` 传入文件路径（非目录）或不存在路径 | `crates/agent/src/service/session_ops.rs:188-221` |
| Medium | `display_name_from_working_dir` 根路径和尾部分隔符 | `crates/agent/src/service/session_ops.rs:223-229` |
| Low | `convert_events_to_messages` ToolCall 无匹配 ToolResult 的场景（取消中断） | `crates/agent/src/service/replay.rs:96-105` |
| Low | `serverAuth.ts` 缺少 bootstrap 超时和 fetch 失败的测试 | `frontend/src/lib/serverAuth.test.ts` |
| Low | `PromptTemplate::render` 未闭合和空占位符错误路径 | `crates/agent/src/prompt/template.rs:15-46` |
| Low | `grep` 工具不存在的路径、`list_dir` 缺少取消和截断测试 | `crates/tools/src/tools/grep.rs`, `list_dir.rs` |

---

## 🏗️ Architecture

| Sev | Inconsistency | Files |
|-----|--------------|-------|
| **Medium** | 前端依赖 `/__astrcode__/run-info` 路由进行浏览器模式认证，但 server 未提供该路由 | `frontend/src/lib/serverAuth.ts:17`, `crates/server/src/main.rs` |
| **Medium** | `RunInfo` 结构体在 server 和 Tauri 间不同步：server 多 `started_at` 字段 | `crates/server/src/main.rs:129-136`, `src-tauri/src/main.rs:33-39` |
| **Medium** | `handle.rs` 引用不存在的 `AgentRuntime`，整个文件是架构变更前的残留死代码 | `src-tauri/src/handle.rs` |
| Low | `SessionMessageDto.ToolCall` 缺少 `error` 字段，前端类型声明有但后端不发送 | `crates/server/src/dto.rs`, `frontend/src/types.ts` |
| Low | `duration_ms` 在 core(`u128`) 和 agent events(`u64`) 间类型不一致 | `crates/core/src/action.rs`, `crates/agent/src/events.rs` |
| Low | `resolve_home_dir` 在 3 个 crate 中重复 4 次，行为略有差异 | `agent/config.rs`, `agent/event_log/paths.rs`, `server/main.rs`, `tauri/paths.rs` |
| Low | CLAUDE.md 引用不存在的 `crates/contracts` | `CLAUDE.md` |
| Low | 前端 4 个 Action 类型声明了但 reducer 无对应 case（死代码） | `frontend/src/types.ts`, `frontend/src/App.tsx` |

---

## 🚨 Must Fix Before Merge

*(Critical/High — 1 Critical, 6 High)*

### Critical

1. **[SEC-001]** `resolve_path` 无工作目录边界检查 — `crates/tools/src/tools/fs_common.rs:20-28`
   - Impact: LLM 可通过绝对路径或 `../` 遍历读写执行任意文件系统路径
   - Fix: 解析路径后用 `canonicalize()` 验证结果在 `ctx.working_dir` 内

### High

2. **[SEC-002]** `findFiles` glob 模式可覆盖 root 目录 — `crates/tools/src/tools/find_files.rs:59-66`
   - Impact: LLM 可枚举任意目录文件
   - Fix: 验证 pattern 不含绝对路径和 `..`，或验证所有 glob 匹配结果在工作目录内

3. **[SEC-003]** symlink 绕过路径沙箱 — `crates/tools/src/tools/fs_common.rs:20-28`
   - Impact: 通过 `working_dir` 内的 symlink 指向并读写外部文件
   - Fix: 在 `resolve_path` 中对最终路径调用 `fs::canonicalize()`

4. **[SEC-004]** shell 工具允许 LLM 指定任意 shell 程序 — `crates/tools/src/tools/shell.rs:156-181`
   - Impact: LLM 可选择任意可执行文件解释命令
   - Fix: 将 `shell` 参数限制为预定义白名单（如 `["pwsh", "powershell", "/bin/sh", "/bin/bash"]`）

5. **[TEST-001]** Shell 工具缺少取消路径测试 — `crates/tools/src/tools/shell.rs`
   - Impact: 取消是用户安全中止失控命令的唯一手段，未测试可能意味着该功能不工作
   - Fix: 添加 cancel、非零退出码、stderr 捕获、spawn 失败等测试

6. **[TEST-002]** `ToolRegistry::execute()` 未知工具名和执行失败分支未测试 — `crates/agent/src/tool_registry.rs:59-82`
   - Impact: agent loop 错误恢复的关键分支未验证
   - Fix: 添加 unknown tool name 和 tool execution error 的测试用例

7. **[TEST-003]** `normalize_session_id` / `normalize_working_dir` 无测试 — `crates/agent/src/service/session_ops.rs:180-221`
   - Impact: 路径注入防御函数未验证
   - Fix: 添加 strip prefix、trim、相对路径拼接、不存在路径、文件非目录等测试

---

## 📎 Pre-Existing Issues (not blocking)

- `crates/ipc/` 已被删除但旧引用可能残留
- `resolve_home_dir` 重复实现（4 处）— 维护负担，变更时容易遗漏
- `contracts` crate 已合并到 `server/dto.rs` 但 CLAUDE.md 文档未同步
- 上次审查中发现的编译警告（dead code）尚未清理
- 部分 `src-tauri/src/handle/` 旧模块路径在历史文档/PR 描述中仍被引用

---

## 🤔 Low-Confidence Observations

- Token 常量时间比较：localhost-only 绑定已大幅降低风险，优先级低
- `resolve_api_key` 启发式规则：代码中已有文档说明设计权衡，属于已知取舍
- `duration_ms` 类型差异：`u128` → `u64` 截断需 ~5.84 亿年才触发，实际无影响
- `read_last_timestamp` 跳过 Error 事件：对用户体验影响极小
- `run.json` 文件权限问题在 Windows 上影响较小（Windows 默认用户隔离）
- `append_and_broadcast_blocking` 嵌套反模式：broadcast channel 满时 `send` 返回值被 `let _ =` 忽略，实际死锁风险较低
