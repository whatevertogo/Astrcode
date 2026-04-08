# Code Review — branch `003-subagent-child-sessions`

## Summary
Files reviewed: ~50 (Rust backend + TypeScript frontend, 仅 .rs/.ts/.tsx/.css) | New issues: 13 (0 Critical, 7 High, 4 Medium, 2 Low) | Perspectives: 4/4

---

## 🔒 Security
| Sev | Issue | File:Line | Attack path |
|-----|-------|-----------|-------------|
| High | 协作工具 (`send_to_child`/`wait_for_child`/`close_child`/`resume_child`) 缺少所有权验证 — 任意 agent 可操作其他 agent | `subagent.rs:824-1060` | LLM agent A 调用 `sendAgent({ agentId: "agent-B-id" })`，可向非子 agent B 注入消息、关闭/恢复 B |
| Medium | SSE 错误事件中暴露 `session_id` 内部标识符 | `stream.rs` 4 处 | 本地 localhost 场景下风险有限，但违反最小信息暴露原则 |

### [SEC-001] HIGH — 协作工具缺少所有权验证

`send_to_child`/`wait_for_child`/`close_child`/`resume_child` 接受 LLM 工具调用传入的 `agent_id`，直接操作目标 agent 而不验证调用者是否为其父级。`_ctx` 参数带下划线前缀（未使用）。

**为什么不是误报：** 同文件 `deliver_to_parent` (line 1051) 正确执行了祖先链验证，证明该模式已存在且此处是疏漏。

**Fix:** 在四个方法中添加 `ctx.agent_context().agent_id == target.parent_agent_id` 检查。

---

## 📝 Code Quality
| Sev | Issue | File:Line | Consequence |
|-----|-------|-----------|-------------|
| High | `reactivate_parent_agent_if_idle` 存在 TOCTOU 竞态 | `mod.rs:143-164` | 两个并发子 agent 完成可同时触发父 agent 重激活，产生重复 turn |
| High | `cancel_for_parent_turn` 从 `complete_session_execution` 被静默删除 | `orchestration.rs:57-62` | 前台（阻塞型）子 agent 在父 turn 结束时不会被取消，变成孤儿 |
| Medium | `CacheTracker::check_and_update` 每次调用做冗余 `String` 分配 | `cache_tracker.rs:72-76` | 不必要的性能开销 |
| Medium | `AnthropicCacheControl::with_ttl("1h")` 发送未文档化的 `ttl` 字段 | `anthropic.rs:510-512` | 可能导致 API 错误或静默失效 |

### [QUAL-001] HIGH — TOCTOU 竞态

`reactivate_parent_agent_if_idle` 读取 `state.running`，然后单独调用 `submit_prompt`。两次读取之间另一子 agent 完成也可通过相同检查，导致两次 submit。

**Fix:** 对 `state.running` 使用 `compare_exchange` 原子操作，或用 mutex 序列化重激活路径。

### [QUAL-002] HIGH — 前台子 agent 孤儿问题

之前的 `complete_session_execution` 调用 `agent_control.cancel_for_parent_turn(turn_id)` 清理子 agent。此调用被完全删除。注释仅提到 "background subrun lifecycle is managed separately"，但前台阻塞型子 agent 在父 turn 异常结束时将无清理路径。

**Fix:** 恢复 `cancel_for_parent_turn` 调用，限定为前台（非 background）子 agent。

---

## ✅ Tests
**整体评价**: 30+ 新测试已添加，覆盖面总体良好。以下为关键缺口：

| Sev | Untested scenario | Location |
|-----|------------------|----------|
| High | `project_child_terminal_delivery` 的 `SubRunOutcome::TokenExceeded` 分支无测试 | `status.rs:83` |
| High | `terminal_summary_or_fallback` 的 `failure.display_message` 回退路径无测试 | `status.rs:113-130` |
| Medium | `reactivate_parent_agent_if_idle` 的重激活/跳过路径均无测试 | `mod.rs:134-191` |
| Low | `normalize_session_id_for_compare` 的边界情况未测试 | `query.rs:92-95` |

---

## 🏗️ Architecture
| Sev | Inconsistency | Files |
|-----|--------------|-------|
| High | 后端发射 `childSessionNotification` SSE 事件，但前端 `normalizeAgentEvent()` 无处理器，所有通知被丢弃为 `unknown event type` | `event.rs` → `agentEvent.ts` → `types.ts` |
| High | `subagent.rs` 从 478 行膨胀到 1239 行，超出宪法 800 行限制 | `subagent.rs` |
| Medium | `mapper.rs` 从 944 行增长到 1033 行（预先存在违规，本 diff 增加 89 行） | `mapper.rs` |
| Medium | `normalize_session_id_for_compare` 与 `validate_session_path_id` 标准化逻辑不一致 | `query.rs` vs `filter.rs` |

### [ARCH-001] HIGH — 前后端合约断裂

后端新增 `AgentEventPayload::ChildSessionNotification`，序列化为 `{"event": "childSessionNotification", ...}`。但前端：
1. `normalizeAgentEvent()` 无此事件分支 → 全部走 `invalidEvent("unknown event type: childSessionNotification")`
2. `AgentEventPayload` union type 缺少此变体

**Impact:** 整个子会话通知管道对前端不可见。

**Fix:** 在 `types.ts` 添加类型变体，在 `agentEvent.ts` 添加处理分支。

### [ARCH-002] HIGH — 文件超限

`subagent.rs` (1239 行) 违反宪法 "runtime facade 单文件不超过 800 行"。

**Fix:** 将子会话交付逻辑提取到 `child_delivery.rs`。

---

## 🚨 Must Fix Before Merge (7 个 High)

1. **[SEC-001]** 协作工具缺少所有权验证 → 添加 `parent_agent_id` 匹配检查
2. **[QUAL-001]** TOCTOU 竞态 → 使用原子 compare-and-swap
3. **[QUAL-002]** 前台子 agent 孤儿 → 恢复 scoped cancel
4. **[ARCH-001]** 前端未实现 `childSessionNotification` → 添加类型和处理分支
5. **[ARCH-002]** `subagent.rs` 超出 800 行 → 拆分子模块
6. **[TEST-001]** `TokenExceeded` 分支未测试 → 添加专项测试
7. **[TEST-002]** `terminal_summary_or_fallback` 回退路径未测试 → 添加专项测试

---

## ✅ Confirmed Safe
- 路径遍历：`validate_session_path_id` 白名单有效
- 新端点认证：`require_auth` 已调用
- IDOR：`child_session_view` 从父 session 历史读取，无越权
- 前端 XSS：React auto-escape，无 `dangerouslySetInnerHTML`
- 依赖边界：无 `protocol`↔`core` 违规，`runtime-*` 不依赖 `runtime`
- DTO 映射：跨层走显式 DTO + mapper
- Serde flatten：`ChildSessionNotification` 无字段冲突

---

## 📎 Pre-Existing Issues (not blocking)
- `mapper.rs` 已超过 800 行（master 上 944 行）
- `anthropic.rs` 已 1596 行

## 🤔 Low-Confidence Observations
- `with_ttl("1h")` 需验证 Anthropic API 是否实际支持，如支持则无问题
- `CacheTracker` String 分配是性能优化点，非正确性问题
- `resume_child_session` ~200 行 async 方法无直接单元测试
