# Agent as Tool (子代理系统) 设计文档

## 1. 概述

Agent as Tool 允许主 Agent 通过 `spawnAgent` 工具委派任务给专门的子 Agent。子 Agent 在受控的隔离环境中执行，拥有独立的工具集、策略和执行限制，最终返回摘要结果给父 Agent。

### 核心价值

- **专业性**: 每个 Agent 专注于特定任务类型（探索、规划、执行、审查）
- **安全性**: 子 Agent 默认只读，权限可被精确控制
- **效率**: 子 Agent 可使用更小/更快的模型，节省成本
- **可观测性**: 完整的事件链追踪，支持嵌套执行

## 2. 架构设计

```
┌─────────────────────────────────────────────────────────────┐
│                    主 Agent Loop                             │
│  ┌──────────────────────────────────────────────────────┐   │
│  │ LLM 决定需要委派任务                                   │   │
│  │ Tool Call: spawnAgent(type="explore",                │   │
│  │                      description="inspect auth",      │   │
│  │                      prompt="inspect auth module")    │   │
│  └──────────────────────┬───────────────────────────────┘   │
└─────────────────────────┼───────────────────────────────────┘
                          ▼
┌─────────────────────────────────────────────────────────────┐
│                    SpawnAgentTool                             │
│  ├─ 解析 SpawnAgentParams                                   │
│  ├─ 暴露稳定 schema + 稳定使用说明                         │
│  ├─ 委托给 SubAgentExecutor.launch()                      │
│  └─ 返回 ToolExecutionResult                              │
└──────────────────────┬──────────────────────────────────────┘
                       ▼
┌─────────────────────────────────────────────────────────────┐
│              AgentExecutionServiceHandle                    │
│  ├─ 查找已注册的 AgentProfile                             │
│  ├─ 验证 AgentMode::SubAgent                              │
│  ├─ 准备执行上下文 (prepare_scoped_execution)             │
│  │   ├─ 解析 SubagentContextOverrides                     │
│  │   ├─ 构建子 Agent 状态                                 │
│  │   └─ 配置独立的 Policy/Tool Set                        │
│  ├─ 通过 AgentControlPlane spawn 子 Agent                 │
│  ├─ 执行子 Agent Loop (ChildExecutionTracker)             │
│  ├─ 收集结果 (SubRunResult)                               │
│  └─ 发送 SubRunStarted/SubRunFinished 事件                │
└─────────────────────────────────────────────────────────────┘
```

## 3. 核心数据模型

### 3.1 AgentProfile (Agent 画像定义)

定义文件: `crates/core/src/agent/mod.rs`

```rust
/// Agent 画像定义。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentProfile {
    /// Profile 唯一标识 (如 "explore", "plan")。
    pub id: String,
    /// 人类可读名称。
    pub name: String,
    /// 作用说明，供路由/提示词/UI 复用。
    pub description: String,
    /// 该 profile 允许的使用模式。
    pub mode: AgentMode,
    /// 子 Agent 专用系统提示，可为空。
    pub system_prompt: Option<String>,
    /// 允许使用的工具集合；为空表示由上层策略决定。
    pub allowed_tools: Vec<String>,
    /// 显式禁止的工具集合。
    pub disallowed_tools: Vec<String>,
    /// 最大 step 数上限。
    pub max_steps: Option<u32>,
    /// token 预算上限。
    pub token_budget: Option<u64>,
    /// 模型偏好。
    pub model_preference: Option<String>,
}
```

**字段说明:**

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | String | 唯一标识符，从 `name` 自动规范化生成 (小写 + 连字符) |
| `name` | String | 用户定义的显示名称 |
| `description` | String | Agent 用途描述，LLM 用于路由决策 |
| `mode` | AgentMode | `Primary` / `SubAgent` / `All`，控制可用场景 |
| `system_prompt` | Option<String> | 子 Agent 的系统提示，Markdown body 或 frontmatter 中的 prompt |
| `allowed_tools` | Vec<String> | 白名单工具集 |
| `disallowed_tools` | Vec<String> | 黑名单工具集（优先级高于白名单） |
| `max_steps` | Option<u32> | 执行步数上限 |
| `token_budget` | Option<u64> | Token 预算上限 |
| `model_preference` | Option<String> | 模型偏好（当前未使用，保留扩展） |

### 3.2 AgentMode (Agent 模式)

```rust
pub enum AgentMode {
    /// 只能作为主 Agent 使用。
    Primary,
    /// 只能作为子 Agent 使用。
    SubAgent,
    /// 主/子 Agent 均可使用。
    All,
}
```

### 3.3 SpawnAgentParams (工具调用参数)

定义文件: `crates/runtime-agent-tool/src/lib.rs`

```rust
/// `spawnAgent` 工具的调用参数。
pub struct SpawnAgentParams {
    /// Agent profile 标识；为空默认 `explore`。
    pub r#type: Option<String>,
    /// 短摘要，只用于 UI / 日志 / 标题。
    pub description: String,
    /// 子 Agent 实际收到的任务正文。
    pub prompt: String,
    /// 可选补充上下文。
    pub context: Option<String>,
}
```

### 3.4 SubagentContextOverrides (上下文覆写)

定义文件: `crates/core/src/agent/mod.rs`

> 说明：这组字段当前用于 root execution API 与内部执行装配，
> **不是** `spawnAgent` 工具的公开参数 schema。

```rust
/// 调用侧可传入的子会话上下文 override。
pub struct SubagentContextOverrides {
    pub storage_mode: Option<SubRunStorageMode>,
    pub inherit_system_instructions: Option<bool>,
    pub inherit_project_instructions: Option<bool>,
    pub inherit_working_dir: Option<bool>,
    pub inherit_policy_upper_bound: Option<bool>,
    pub inherit_cancel_token: Option<bool>,
    pub include_compact_summary: Option<bool>,
    pub include_recent_tail: Option<bool>,
    pub include_recovery_refs: Option<bool>,
    pub include_parent_findings: Option<bool>,
}
```

**当前实现的约束:**

| Override 字段 | 默认值 | 当前行为 |
|--------------|--------|---------|
| `storage_mode` | `SharedSession` | 支持 `SharedSession` / `IndependentSession` |
| `inherit_system_instructions` | `true` | 全继承或不继承，无细粒度拆分 |
| `inherit_project_instructions` | `true` | 全继承或不继承，无细粒度拆分 |
| `inherit_working_dir` | `true` | - |
| `inherit_policy_upper_bound` | `true` | - |
| `inherit_cancel_token` | `true` | **不支持设为 false** - 取消必须级联传播，否则父取消后子 Agent 成为孤儿进程 |
| `include_compact_summary` | `false` | - |
| `include_recent_tail` | `true` | - |
| `include_recovery_refs` | `false` | **不支持设为 true** - 跨会话引用协议未定义 |
| `include_parent_findings` | `false` | **不支持设为 true** - findings 格式非结构化，需先定义过滤机制 |

### 3.5 SubRunResult (执行结果)

```rust
/// 子执行结构化结果。
pub struct SubRunResult {
    pub status: SubRunOutcome,
    pub handoff: Option<SubRunHandoff>,
    pub failure: Option<SubRunFailure>,
}

/// 子执行结果状态。
pub enum SubRunOutcome {
    Running,
    Completed,
    Failed,
    Aborted,
    TokenExceeded,
}

pub struct SubRunHandoff {
    pub summary: String,
    pub findings: Vec<String>,
    pub artifacts: Vec<ArtifactRef>,
}

pub struct SubRunFailure {
    pub code: SubRunFailureCode,
    pub display_message: String,
    pub technical_message: String,
    pub retryable: bool,
}
```

### 3.6 SubRunHandle (运行时句柄)

```rust
/// 受控子会话的轻量运行句柄。
pub struct SubRunHandle {
    pub sub_run_id: String,
    pub agent_id: String,
    pub session_id: String,
    pub child_session_id: Option<String>,
    pub depth: usize,
    pub parent_turn_id: Option<String>,
    pub parent_agent_id: Option<String>,
    pub agent_profile: String,
    pub storage_mode: SubRunStorageMode,
    pub status: AgentStatus,
}
```

### 3.7 AgentEventContext (事件上下文)

```rust
/// turn 级事件的 Agent 元数据。
pub struct AgentEventContext {
    pub agent_id: Option<String>,
    pub parent_turn_id: Option<String>,
    pub agent_profile: Option<String>,
    pub sub_run_id: Option<String>,
    pub invocation_kind: Option<InvocationKind>,
    pub storage_mode: Option<SubRunStorageMode>,
    pub child_session_id: Option<String>,
}
```

## 4. 工具实现

### 4.1 SpawnAgentTool

定义文件: `crates/runtime-agent-tool/src/lib.rs`

```rust
/// 把子 Agent 能力暴露给 LLM 的内置工具。
pub struct SpawnAgentTool {
    launcher: Arc<dyn SubAgentExecutor>,
}

/// 子 Agent 启动器抽象。
#[async_trait]
pub trait SubAgentExecutor: Send + Sync {
    async fn launch(&self, params: SpawnAgentParams, ctx: &ToolContext) -> Result<SubRunResult>;
}

/// 子 Agent profile 目录抽象。
pub trait AgentProfileCatalog: Send + Sync {
    fn list_subagent_profiles(&self) -> Vec<AgentProfile>;
}
```

**设计要点:**

1. **依赖倒置**: `SpawnAgentTool` 不直接依赖 `RuntimeService`，而是通过 `SubAgentExecutor` trait 解耦
2. **稳定 tool surface**: tool description 不再内嵌动态 profile 列表，避免 bootstrap 与热重载后出现陈旧描述
3. **职责拆分**: profile discovery 通过 `AgentProfileCatalog` 单独暴露，供 prompt contributor 或独立索引能力使用
4. **参数验证**: 在工具层做参数解析，失败返回 `ToolExecutionResult { ok: false, error: ... }`
5. **结果格式化**: 返回包含 `outcome`、`findings`、`artifacts` 的 metadata

### 4.2 launch_subagent 实现

定义文件: `crates/runtime/src/service/execution/subagent.rs`

**执行流程:**

1. **前置校验**: 获取 parent_turn_id、event_sink，查找 AgentProfile
2. **模式验证**: 调用 `ensure_subagent_mode()` 确认 profile 可作为子 Agent
3. **准备执行**: 调用 `prepare_scoped_execution()` 解析 overrides 和构建执行规格
4. **存储模式**: 根据 `storage_mode` 决定是否创建独立 session
5. **控制平面**: 通过 `agent_control.spawn_with_storage()` 注册子 Agent
6. **事件发送**: 发送 `SubRunStarted` 事件
7. **执行循环**: 使用 `ChildExecutionTracker` 跟踪步数和 token
8. **结果收集**: 构建 `SubRunResult`，发送 `SubRunFinished` 事件

## 5. Agent Profile 加载

### 5.1 加载器架构

定义文件: `crates/runtime-agent-loader/src/lib.rs`

```
AgentProfileLoader
    ├─ user_agent_dirs: [~/.claude/agents, ~/.astrcode/agents]
    └─ load_for_working_dir(working_dir)
        ├─ 1. builtin agents (内置)
        ├─ 2. ~/.claude/agents (用户级)
        ├─ 3. ~/.astrcode/agents (用户级)
        ├─ 4. <project>/.claude/agents (项目级)
        └─ 5. <project>/.astrcode/agents (项目级)
```

**优先级**: 后者覆盖前者，项目级 > 用户级 > 内置

### 5.2 文件格式

**Markdown + YAML Frontmatter (推荐)**:

```markdown
---
name: review
description: 审查代码的质量、安全性和最佳实践
tools: ["readFile", "grep"]
disallowedTools: ["shell"]
---

重点审查行为回归、边界条件和测试缺口。
```

**纯 YAML**:

```yaml
name: planner
description: 计划任务
tools: ["readFile", "grep"]
systemPrompt: |
  先阅读代码，然后制定计划。
```

### 5.3 工具列表格式

支持两种格式:

```yaml
# YAML 列表
tools: ["readFile", "grep", "glob"]

# CSV 字符串
tools: readFile, grep, glob
```

## 6. 预置 Agent Profiles

定义目录: `crates/runtime-agent-loader/src/builtin_agents/`

| Agent ID | 用途 | 工具集 | 说明 |
|----------|------|--------|------|
| `explore` | 代码探索 | `readFile`, `listDir`, `findFiles`, `grep` | 快速检索和阅读代码，偏向并行搜索 |
| `plan` | 任务规划 | `readFile`, `grep` | 分析需求，输出执行计划，不执行改写 |
| `execute` | 定向执行 | `readFile`, `writeFile`, `editFile`, `shell` | 围绕明确目标做定向实现 |
| `reviewer` | 代码审查 | 只读工具 | 多视角审查（安全、质量、测试、架构） |

### 6.1 explore Agent

**特点:**
- 广度优先搜索策略
- 最大并行化工具调用
- 返回简洁答案，不做全面概述

### 6.2 plan Agent

**特点:**
- 只规划，不实现
- 可调用 explore 子 Agent 进行发现
- 输出结构化计划到 `/memories/session/plan.md`

### 6.3 execute Agent

**特点:**
- 定向修改，保持范围可控
- 结束前做最小必要验证

### 6.4 reviewer Agent

**特点:**
- 四视角审查: 安全、代码质量、测试、架构
- 高置信度过滤，只报告真实问题
- 输出到 `CODE_REVIEW_ISSUES.md`

## 7. 执行隔离与事件关联

### 7.1 隔离机制

| 维度 | 隔离策略 |
|------|---------|
| **工具集** | `allowed_tools` + `disallowed_tools` 白黑名单 |
| **策略** | 继承父策略上界，可进一步收紧 |
| **取消** | 父取消 → 子自动取消 (CancelToken 级联) |
| **存储** | `SharedSession` 或 `IndependentSession` |
| **步数/Token** | `max_steps` / `token_budget` 限制 |

### 7.2 事件关联

```
Turn #5 (用户: "重构 auth 模块")
  → LLM 决定调用 spawnAgent(type="explore")
  → SubRunStarted { sub_run_id, parent_turn_id: "turn-5", agent_profile: "explore" }
  → Turn #5.1 (sub: explore)
    → tool_call: readFile("auth.rs")
    → tool_call: grep("authenticate")
    → tool_result: ...
  → SubRunFinished { result: { summary: "auth 模块有3个核心函数..." } }
  → LLM 继续 Turn #5...
```

**事件链:**
- `SubRunStarted.agent.parent_turn_id` → 父 turn
- `SubRunFinished.result` → 结构化结果

## 8. 可观测性

### 8.1 指标

通过 `observability.record_subrun_execution()` 记录:

- 执行时长
- 结果状态 (completed/failed/aborted/token_exceeded)
- 存储模式
- 步数
- 估算 token

### 8.2 事件流

父 Agent 可通过以下事件消费子执行结果:

- `SubRunFinished.result.summary`
- `SubRunFinished.result.findings`
- `SubRunFinished.result.artifacts`
- `SubRunFinished.step_count`
- `SubRunFinished.estimated_tokens`

## 9. 安全与策略

### 9.1 子 Agent 策略上下文

子 Agent 的策略评估继承父 Agent 的策略上界，并额外检查:

1. **工具白名单**: `allowed_tools` 限制
2. **工具黑名单**: `disallowed_tools` 排除
3. **审批限制**: 子 Agent 通常不能请求用户审批（无 UI 权限）

### 9.2 取消传播

```rust
// CancelToken 级联
let parent_cancel = CancelToken::new();
let child_cancel = parent_cancel.child_token();

// 父取消时，子也自动取消
parent_cancel.cancel();
assert!(child_cancel.is_cancelled());
```

## 10. API 设计 (扩展)

### 10.1 REST API

```
POST /api/v1/sessions              - 创建会话
GET  /api/v1/sessions              - 列出会话
GET  /api/v1/sessions/{id}          - 获取会话详情
DELETE /api/v1/sessions/{id}        - 删除会话

POST /api/v1/sessions/{id}/message  - 发送消息 (流式)
POST /api/v1/sessions/{id}/abort    - 中止执行

GET  /api/v1/agents                 - 列出可用 Agent
POST /api/v1/agents/{id}/execute    - 创建 root execution

GET  /api/v1/tools                  - 列出可用工具
POST /api/v1/tools/{id}/execute     - 执行单个工具
```

### 10.2 Root Execution

允许外部系统创建独立 session 并启动 root execution:

```rust
pub struct AgentExecuteRequestDto {
    pub task: String,
    pub context: Option<String>,
    pub working_dir: Option<String>,
    pub max_steps: Option<u32>,
    pub context_overrides: Option<SubagentContextOverridesDto>,
}
```

## 11. 配置

### 11.1 Agent 配置文件

优先级 (后者覆盖前者):

1. `builtin://` 内置 agents
2. `~/.claude/agents/`
3. `~/.astrcode/agents/`
4. `<working_dir>/.claude/agents/`
5. `<working_dir>/.astrcode/agents/`

### 11.2 示例配置

```markdown
---
name: security-reviewer
description: 专门审查安全问题的 Agent
tools: ["readFile", "grep"]
disallowedTools: ["shell", "writeFile"]
---

你是安全审查专家，重点关注:
- SQL 注入
- XSS
- 硬编码密钥
- 不安全的反序列化
```

## 12. 与竞品对比

| 特性 | Astrcode | Codex | Claude Code |
|------|----------|-------|-------------|
| Agent as Tool | ✅ 内置 Profile 系统 | spawn_agent，无 Profile 管理 | sub-agents |
| Profile 加载 | ✅ 多目录优先级 | - | ✅ |
| 工具白黑名单 | ✅ | - | ✅ |
| 事件嵌套 | ✅ parent_turn_id | - | - |
| 独立 Session | ✅ 实验性支持 | - | - |
| 模型选择 | ⏳ 保留字段 | ✅ | - |
