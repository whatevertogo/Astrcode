# 多 Agent 系统改进 TODO

> 生成日期：2026-04-05
> 基于对 Claude Code、Codex、Kimi-CLI、OpenCode、pi-mono 五个项目的深度对比分析
> 与 `docs/architecture/agent-loop-roadmap.md` 路线图对齐

---

## P0：安全性必须项（立即）

### 0.1 递归深度与并发限制

**现状**：`AgentControl` 没有递归深度上限和并发 Agent 数量限制，存在无限递归和资源耗尽风险。

**参考**：Codex 的 `agent_max_depth` + CAS 无锁并发控制

**TODO**：
- [ ] 在 `AgentControl` 中增加 `max_depth: usize`（默认 3）
- [ ] 在 `AgentControl` 中增加 `max_concurrent: usize`（默认 5）+ `running_count: AtomicUsize`
- [ ] `spawn()` 时检查深度和并发，超限返回 `AgentControlError`
- [ ] 在 `agent_execution.rs` 的 `execute_subagent` 中传递当前深度

**涉及文件**：
- `crates/runtime-agent-loop/src/agent_control.rs`（或未来的 `runtime-agent-control`）
- `crates/runtime/src/service/agent_execution.rs`

```rust
// 预期签名变更
pub struct AgentControl {
    max_depth: usize,
    max_concurrent: usize,
    running_count: Arc<AtomicUsize>,
    // ... 现有字段
}

pub enum AgentControlError {
    ParentAgentNotFound { agent_id: String },
    MaxDepthExceeded { current: usize, max: usize },
    MaxConcurrentExceeded { current: usize, max: usize },
}
```

---

## P1：Agent Profile 完整性（短期）

### 1.1 `model_preference` 实际生效

**现状**：`AgentProfile.model_preference: Option<String>` 已定义但从未被消费，子 Agent 始终使用主 Agent 的模型。

**参考**：Claude Code 的 `model: sonnet` / `effort: high`；Codex 的角色级 `model = "gpt-4o"`

**TODO**：
- [ ] 在 `agent_execution.rs` 的 `build_agent_loop` 中消费 `profile.model_preference`
- [ ] `ConfigFileProviderFactory` 支持按 profile 覆盖模型选择
- [ ] 内置 Profile 指定模型偏好（如 explore 用 haiku，review 用 sonnet）

**涉及文件**：
- `crates/runtime/src/service/agent_execution.rs`
- `crates/runtime/src/provider_factory.rs` 或 `crates/runtime-agent-loop/src/provider_factory.rs`

### 1.2 Profile 增加 `inherit_rules` 控制

**现状**：子 Agent 默认继承全部 AGENTS.md 规则，无法控制哪些规则传递给子 Agent。

**参考**：Claude Code 的 `omitClaudeMd: true`（只读 agent 不继承主提示以节省 token）

**TODO**：
- [ ] `AgentProfile` 增加 `inherit_rules: Option<bool>`（默认 true）
- [ ] `build_child_prompt_declarations` 根据此字段决定是否注入 AGENTS.md 规则
- [ ] 内置只读 Profile（explore/review）默认 `inherit_rules: false`

**涉及文件**：
- `crates/core/src/agent/mod.rs`
- `crates/runtime/src/service/agent_execution.rs`
- `crates/runtime-agent-loader/src/lib.rs`

### 1.3 Profile 增加 `isolation` 模式

**现状**：子 Agent 与主 Agent 共享同一个工作目录和文件系统。

**参考**：Claude Code 的 `isolation: worktree`；Codex 的沙箱隔离

**TODO**：
- [ ] `AgentProfile` 增加 `isolation: AgentIsolation` 枚举（`None` / `Worktree`）
- [ ] `Worktree` 模式下自动创建临时 git worktree
- [ ] 子 Agent 执行结束后自动清理 worktree

**涉及文件**：
- `crates/core/src/agent/mod.rs`
- `crates/runtime/src/service/agent_execution.rs`

```rust
#[derive(Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AgentIsolation {
    #[default]
    None,
    Worktree,
}
```

---

## P2：Agent 协作能力（中期）

### 2.1 Agent 间消息传递

**现状**：`runAgent` 是单向的——主 Agent 调用子 Agent，等待结果。子 Agent 无法主动向主 Agent 报告进展。

**参考**：
- Claude Code：`SendMessage` 工具 + 邮箱系统
- Codex：`Mailbox`（带序列号、条件触发、非阻塞发送）
- Kimi-CLI：`Wire` 双向队列

**TODO**：
- [ ] 在 `runtime-agent-tool` 中增加 `SendMessage` 工具定义
- [ ] 在 `AgentControl` 中维护 per-agent 消息队列
- [ ] `SendMessage` 参数：`to: String`（目标 agent ID）、`message: String`
- [ ] 主 Agent 通过 `WaitForMessage` 或事件流接收子 Agent 消息

**涉及文件**：
- 新增 `crates/runtime-agent-tool/src/send_message.rs`
- `crates/runtime-agent-loop/src/agent_control.rs`
- `crates/runtime/src/service/agent_execution.rs`

### 2.2 Hook 系统增加 `block` 语义

**现状**：`HookRuntime` 支持 `before_tool_call` / `after_turn` / `pre/post_compact`，但 Hook 只能修改参数，不能阻止操作执行。

**参考**：pi-mono 的事件驱动扩展系统（`return { block: true, reason: "..." }`）

**TODO**：
- [ ] `HookResult` 增加 `Block { reason: String }` 变体
- [ ] `tool_cycle.rs` 在 `before_tool_call` Hook 返回 `Block` 时跳过执行
- [ ] 增加事件类型：`agent_spawned`、`agent_completed`、`budget_exceeded`
- [ ] 支持自定义压缩逻辑（Hook 返回自定义摘要）

**涉及文件**：
- `crates/runtime-agent-loop/src/hook_runtime.rs`
- `crates/runtime-agent-loop/src/agent_loop/tool_cycle.rs`

### 2.3 审批路由回根会话

**现状**：子 Agent 的 `SubAgentPolicyEngine` 对需要审批的调用直接 deny。

**参考**：Codex 的 `forward_events()` 将审批请求路由到父会话；Kimi-CLI 的根交互面统一审批

**TODO**：
- [ ] `SubAgentPolicyEngine` 的 `Ask` 分支改为将审批请求发回父 turn
- [ ] 父 turn 的 event_sink 发出 `ApprovalRequest` 事件
- [ ] 审批结果通过消息队列回传给子 Agent
- [ ] 如果当前场景不支持审批 UI，则回退到 deny

**涉及文件**：
- `crates/runtime/src/service/agent_execution.rs`（`SubAgentPolicyEngine`）

---

## P3：生态集成（中长期）

### 3.1 ACP 协议支持

**现状**：Astrcode 有自己的 SSE 协议，无法与 IDE 原生集成。

**参考**：Kimi-CLI 的完整 ACP 实现（Wire → ACP 消息映射）

**TODO**：
- [ ] 在 `protocol` crate 中增加 ACP DTO 层
- [ ] 实现 `StorageEvent → ACP Event` 转换器
- [ ] Server 增加 ACP 端点（`/acp/initialize`、`/acp/prompt`、`/acp/event`）
- [ ] 支持 ACP 权限请求映射（`ApprovalRequest → request_permission`）

**涉及文件**：
- 新增 `crates/protocol/src/acp/`
- `crates/server/src/routes/`

**ACP 事件映射**：
| Astrcode StorageEvent | ACP Event |
|----------------------|-----------|
| `AssistantDelta` | `AgentMessageChunk` |
| `ThinkingDelta` | `AgentThoughtChunk` |
| `ToolCall` | `ToolCallStart` |
| `ToolResult` | `ToolCallProgress` |
| `TurnDone` | `TurnEnd` |

### 3.2 动态工具注册

**现状**：工具在 bootstrap 时注册，运行时不可动态添加。

**参考**：pi-mono 的 `pi.registerTool()`；Claude Code 的 MCP 服务器动态加载

**TODO**：
- [ ] `CapabilityRouter` 支持运行时 `register` / `unregister`
- [ ] 工具热重载与插件生命周期绑定
- [ ] 子 Agent 可按 Profile 获得不同的 MCP 工具集

**涉及文件**：
- `crates/core/src/registry/router.rs`
- `crates/plugin/`

### 3.3 控制平面可观测性

**现状**：`AgentControl` 没有暴露任何指标。

**参考**：Codex 的 `AgentStatus` 枚举 + 状态订阅（`subscribe_status`）；OpenCode 的 `BusEvent`

**TODO**：
- [ ] `AgentControl` 暴露活跃/终态分布计数
- [ ] 记录取消来源（用户取消 / 父取消 / 预算耗尽 / 步数超限）
- [ ] 记录失败类型分类
- [ ] 通过 `RuntimeObservability` 聚合指标

**涉及文件**：
- `crates/runtime-agent-loop/src/agent_control.rs`
- `crates/runtime/src/service/observability.rs`

---

## 架构调整建议（与 roadmap 对齐）

以下是对 `agent-loop-roadmap.md` 中路线图的补充建议：

### AgentControl 应独立为 `runtime-agent-control` crate

Roadmap 已在"推荐落点"中提到这一点。这是正确的方向：

**理由**：
- `AgentControl` 是纯数据结构 + async 状态管理，和 LLM 调用/工具执行无关
- 放在 `runtime-agent-loop` 里导致改注册表逻辑要重编译整个执行引擎
- 独立后可被 `runtime`、`runtime-agent-loop`、未来 `runtime-agent-tool` 共同依赖

**TODO**：
- [ ] 创建 `crates/runtime-agent-control/`
- [ ] 从 `crates/runtime-agent-loop/src/agent_control.rs` 迁移
- [ ] 更新 `Cargo.toml` 依赖关系

### SubAgentPolicyEngine 和 ChildExecutionTracker 应移到 `runtime-agent-loop`

**现状**：两者都在 `crates/runtime/src/service/agent_execution.rs` 中，但它们实现了 `core` trait 或消费了 `runtime-agent-loop` 的导出函数。

**理由**：
- `SubAgentPolicyEngine` 实现了 `PolicyEngine` trait（core 层），应靠近 trait 定义
- `ChildExecutionTracker` 调用了 `estimate_text_tokens`（runtime-agent-loop 的导出），形成跨 crate 调用
- `agent_execution.rs` 目前 25KB，职责过多

**TODO**：
- [ ] `SubAgentPolicyEngine` → `crates/core/src/policy/subagent.rs` 或 `crates/runtime-agent-loop/src/`
- [ ] `ChildExecutionTracker` → `crates/runtime-agent-loop/src/agent_loop/`
- [ ] `agent_execution.rs` 只保留编排逻辑（spawn → 配置 → 执行 → 收集结果）

---

## 不建议做的事

> 与 roadmap §7 对齐

1. **不建议先做 D-Mail**：需要 checkpoint/revert 和持久化 message identity 基础
2. **不建议让子 Agent 直接共享主上下文**：会带来 token 污染、审批混乱、不可解释的 revert
3. **不建议把 Agent registry 塞进 Capability registry**：工具是能力调度，Agent 是生命周期管理，两者应并列
4. **不建议先上平台沙箱**：先统一 orchestrator 抽象 + 审批缓存 + 风险分类，再做平台隔离
5. **不建议先做"大而全开放 API"**：没有 turn status、child session、abort 语义之前，API 只是暴露不稳定状态

---

## 优先级总结

| 优先级 | 项目 | 工作量 | 价值 |
|--------|------|--------|------|
| **P0** | 递归深度 + 并发限制 | 小 | 安全性必须 |
| **P1** | `model_preference` 生效 | 小 | Profile 完整性 |
| **P1** | `inherit_rules` 控制 | 小 | Token 优化 |
| **P1** | `isolation` 模式 | 中 | 安全隔离 |
| **P1** | AgentControl 独立 crate | 中 | 编译隔离 |
| **P2** | Agent 间消息传递 | 中 | 协作能力 |
| **P2** | Hook `block` 语义 | 中 | 安全控制 |
| **P2** | 审批路由回根会话 | 中 | 审批一致性 |
| **P3** | ACP 协议层 | 大 | IDE 集成 |
| **P3** | 动态工具注册 | 大 | 扩展生态 |
| **P3** | 控制平面可观测性 | 中 | 运维可见 |
