# Code Review — master (staged, +2008 lines)

## Summary
Files reviewed: 32 | New issues: 7 (0 critical, 1 high, 4 medium, 2 low) | Perspectives: 4/4

本次变更实现"子 Agent 执行系统"：新增 `runtime-agent-tool` crate 提供 `runAgent` 工具；runtime 层添加 `DeferredSubAgentExecutor`（`Weak<RuntimeService>` 延迟绑定）和 `AgentExecutionService`；server 层预埋 `/api/v1/agents` 和 `/api/v1/tools` 端点；前端在所有消息类型中添加 `agentId`/`parentTurnId`/`agentProfile` 字段并实现子 Agent 消息分组 UI。

---

## 🔒 Security

*No security issues found.*

- 所有新端点均通过 `require_auth()` 守卫
- `RunAgentParams.name` 仅作为 `BTreeMap` key 查找 profile，不触及 shell/SQL/FS
- `subset_for_tools` 的工具名来自 profile 配置而非直接用户输入
- 前端无 `dangerouslySetInnerHTML`，React 默认转义

---

## 📝 Code Quality

| Sev | Issue | File:Line | Consequence |
|-----|-------|-----------|-------------|
| Medium | `step_index >= max_steps` 允许额外一步部分执行 | [subagent.rs:122](crates/runtime-agent-loop/src/subagent.rs#L122) | `max_steps: 1` 实际会开始执行第 2 步（step_index=1）才取消，与"恰好 N 步"的直觉不符 |

**详细说明：**

`step_index` 从 0 开始计数。`PromptMetrics` 在每步 LLM 调用前发出。当 `max_steps=1` 时：
- step_index=0：正常完成（第 1 步）
- step_index=1：`1 >= 1` 触发取消，但 LLM 请求已发出（第 2 步开始后才被中止）

这是协作式取消的固有延迟，但"max_steps"语义需明确。建议改为 `*step_index + 1 > max_steps` 或在文档中说明 max_steps 是"完成后最多取消"的软上限。

---

## ✅ Tests

**Run results**: 测试运行中

| Sev | Untested scenario | Location |
|-----|------------------|----------|
| High | `SubAgentPolicyEngine::check_capability_call` 三个分支均无测试 | [subagent.rs:56-72](crates/runtime-agent-loop/src/subagent.rs#L56-L72) |
| Medium | `CapabilityRouter::subset_for_tools` 无测试——子 Agent 工具可见性的安全边界 | [router.rs:449-469](crates/core/src/registry/router.rs#L449-L469) |
| Medium | `resolve_profile_tool_names` allow/deny 交集逻辑无直接测试 | [agent_execution.rs:407-438](crates/runtime/src/service/agent_execution.rs#L407-L438) |
| Medium | `DeferredSubAgentExecutor` 绑定后成功路径、Arc drop 后失败路径无专门测试 | [agent_execution.rs:67-104](crates/runtime/src/service/agent_execution.rs#L67-L104) |
| Low | `ChannelToolEventSink` 无独立单元测试（被集成测试间接覆盖） | [tool_cycle.rs:58-68](crates/runtime-agent-loop/src/agent_loop/tool_cycle.rs#L58-L68) |

---

## 🏗️ Architecture

*No architecture issues found.*

- DTO ↔ Domain 映射完整对称（`AgentProfileDto` ↔ `AgentProfileSummary`，`ToolDescriptorDto` ↔ `ToolSummary`）
- 前端类型与后端 SSE 事件格式一致（`AgentContextDto` 通过 `#[serde(flatten)]` 展开）
- `runtime-agent-tool` crate 仅依赖 `core`，符合依赖规则
- API 路径遵循 `/api/v1/` 前缀约定
- `build_agent_loop_from_parts` 在所有调用点签名一致

---

## 🚨 Must Fix Before Merge

*(无 Critical/High 阻塞项。以下 High 建议优先处理：)*

1. **[TEST-001]** `SubAgentPolicyEngine::check_capability_call` 零测试覆盖
   - Impact: 子 Agent 的核心安全策略（工具白名单 + 禁止审批）完全未经验证
   - Fix: 添加 3 个测试：(a) 不在白名单的工具被 deny；(b) 在白名单的工具正确委托父策略；(c) 父策略返回 `Ask` 时被转为 `Deny`

---

## 📎 建议修复（不阻塞合并）

1. **[QUAL-001]** 明确 `max_steps` 语义——在 `AgentProfile.max_steps` 字段注释中说明"允许 N 步完成后取消第 N+1 步"，或改为 `*step_index + 1 > max_steps` 实现精确限制
2. **[TEST-002]** 给 `CapabilityRouter::subset_for_tools` 补测试（安全敏感方法）
3. **[TEST-003]** 给 `resolve_profile_tool_names` 补 allow/deny 交集测试
4. **[TEST-004]** 给 `DeferredSubAgentExecutor` 补 bind-success 和 Arc-drop 场景测试

---

## 📎 Pre-Existing Issues (not blocking)
- `StdRwLock` `.expect()` on poison 在整个 codebase 中一致使用，非本次引入

## 🤔 Low-Confidence Observations
- `max_steps` 的 off-by-one 可能是设计意图（协作式取消的软限制），但建议在文档中明确
- `subset_for_tools` 使用 `StdRwLock::read().unwrap()` 与 codebase 一致，但如果在持有读锁期间发生 panic 会 poison 锁
