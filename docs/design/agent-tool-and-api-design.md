## 4. 基本一: Agent as Tool (子代理系统)

### 4.1 架构设计

```
AgentLoop (主循环)
    ↓ LLM 决定需要委派任务
    ↓
Tool Call: runAgent(name="explore", task="查找所有使用 X 的地方")
    ↓
AgentToolExecutor
    ├─ 查找已注册的 Agent Profile
    ├─ 创建子 Agent Loop (独立的 AgentLoop 实例)
    ├─ 配置独立的:
    │  ├─ Prompt (任务描述 + 上下文)
    │  ├─ Policy (子 Agent 权限, 可能是主 Agent 的子集)
    │  ├─ Tool Set (可用工具集, 通常只读)
    │  └─ Token Budget (防止子 Agent 消耗过多)
    ├─ 执行子 Agent Loop
    ├─ 收集结果 (摘要 + 关键发现)
    └─ 返回 ToolResult 给主 Agent
```

### 4.2 数据模型

#### Agent Profile (Agent 配置)

```rust
/// Agent Profile 定义
pub struct AgentProfile {
    /// Agent 唯一标识 (如 "explore", "plan", "refactor")
    pub id: String,
    /// 显示名称
    pub name: String,
    /// 描述 (用于 LLM 理解何时调用此 Agent)
    pub description: String,
    /// 角色/系统提示
    pub system_prompt: Option<String>,
    /// 工具集限制 (只允许使用的工具)
    pub allowed_tools: Option<Vec<String>>,
    /// 最大步数限制
    pub max_steps: usize,
    /// Token 预算
    pub token_budget: Option<usize>,
    /// 模型偏好 (可能使用更小/更快的模型)
    pub model_preference: Option<String>,
    /// 策略覆盖
    pub policy_override: Option<PolicyOverride>,
}
```

#### Agent Tool 定义

```rust
/// runAgent 工具的实现
pub struct RunAgentTool {
    profiles: Arc<Vec<AgentProfile>>,
    runtime: Arc<RuntimeService>,
}

impl Tool for RunAgentTool {
    async fn invoke(&self, params: RunAgentParams, ctx: &ToolContext) -> Result<ToolExecutionResult> {
        // 1. 查找 Profile
        let profile = self.find_profile(&params.name)
            .ok_or_else(|| NotFoundError::new(...))?;
            
        // 2. 创建子 Agent Loop
        let sub_agent = SubAgentLoop::new(
            profile.clone(),
            params.task.clone(),
            ctx.working_dir.clone(),
        );
        
        // 3. 执行
        let result = sub_agent.run(cancel_token).await;
        
        // 4. 返回摘要 (而非完整输出, 节省token)
        ToolExecutionResult::ok(Self::summarize_result(&result))
    }
}
```

#### Agent 调用参数

```rust
pub struct RunAgentParams {
    /// Agent 名称
    pub name: String,
    /// 任务描述 (会作为子 Agent 的用户消息)
    pub task: String,
    /// 额外上下文 (可选)
    pub context: Option<String>,
    /// 覆盖最大步数
    pub max_steps: Option<usize>,
    /// 子会话上下文 override
    pub context_overrides: Option<SubagentContextOverrides>,
}
```

#### 当前 `contextOverrides` 契约

Astrcode 当前坚持“有限 override”，不会开放 Claude Code 式的自由共享父状态模型。

当前已稳定支持：

- `storageMode`
- `includeCompactSummary`
- `includeRecentTail`

当前明确拒绝并返回错误：

- `inheritSystemInstructions` 与 `inheritProjectInstructions` 解析后不一致
- `inheritCancelToken=false`
- `includeRecoveryRefs=true`
- `includeParentFindings=true`

当前继续保守处理：

- `inheritSystemInstructions` / `inheritProjectInstructions` 仍按“全继承 / 不继承”处理，
  暂不对 prompt declarations 做更细粒度拆分
- `independentSession` 继续受运行时 experimental 开关控制

### 4.3 预置 Agent Profiles

基于竞品分析, 推荐预置以下 Agent:

| Agent ID | 用途 | 工具集 | 最大步数 | 说明 |
|----------|------|--------|----------|------|
| `explore` | 代码探索 | 只读工具 | 5 | 读取文件、搜索、理解代码 |
| `plan` | 任务规划 | 只读 + 思考 | 3 | 分析需求, 输出执行计划 |
| `execute` | 代码执行 | 读写 + Shell | 10 | 执行具体的代码修改 |
| `review` | 代码审查 | 只读 | 5 | 审查代码质量、安全问题 |

### 4.4 执行隔离

```rust
/// 子 Agent 执行上下文
pub struct SubAgentContext {
    /// 父 Agent 的 turn_id
    pub parent_turn_id: String,
    /// 子 Agent 自己 turn 的 turn_id
    pub sub_turn_id: String,
    /// 独立的 Event Writer (可选: 写入同一 session 或独立 session)
    pub event_writer: Arc<dyn EventLogWriter>,
    /// 取消令牌 (父 Agent 取消时子 Agent 也取消)
    pub cancel_token: CancelToken,
    /// 父调用上下文
    pub parent_call_id: String,
}
```

### 4.5 事件关联

子 Agent 的执行事件会标记 `parent_turn_id`, 这样在 EventLog 中可以看出嵌套关系:

```
Turn #5 (用户: "重构 auth 模块")
  → LLM 决定调用 runAgent(name="explore")
  → Turn #5.1 (sub: explore)
    → tool_call: readFile("auth.rs")
    → tool_call: grep("authenticate")
    → tool_result: ...
  → turn_done → ToolResult: {"summary": "auth 模块有3个核心函数..."}
  → LLM 继续 Turn #5...
```

---

## 5. 扩展二: 开放 API 设计

### 5.1 API 概览

基于 OpenCode 和 Pi-Mono 的成熟 API 设计, 扩展 Astrcode 的 server crate:

```
POST /api/v1/sessions              - 创建会话
GET  /api/v1/sessions              - 列出会话
GET  /api/v1/sessions/{id}          - 获取会话详情
DELETE /api/v1/sessions/{id}        - 删除会话

POST /api/v1/sessions/{id}/message         - 发送消息 (流式)
POST /api/v1/sessions/{id}/message/async   - 发送消息 (异步)
GET  /api/v1/sessions/{id}/messages        - 获取消息历史
POST /api/v1/sessions/{id}/abort           - 中止执行

POST /api/v1/sessions/{id}/fork            - 分叉会话
POST /api/v1/sessions/{id}/summarize       - 压缩会话
POST /api/v1/sessions/{id}/revert          - 撤销到指定位置

GET  /api/v1/sessions/{id}/events          - 订阅事件 (SSE)
GET  /api/v1/sessions/{id}/events/stream   - WebSocket 事件流

GET  /api/v1/agents                 - 列出可用 Agent
POST /api/v1/agents/{id}/execute     - 创建 root execution
GET  /api/v1/sessions/{id}/subruns/{sub_run_id} - 查询子会话状态

GET  /api/v1/tools                  - 列出可用工具
POST /api/v1/tools/{id}/execute      - 执行单个工具

GET  /health                        - 健康检查
```

### 5.2 核心端点详情

#### POST /api/v1/sessions/{id}/message (流式)

```rust
// 请求
pub struct MessageRequest {
    /// 消息内容 (支持多部分)
    pub content: Vec<MessagePart>,
    /// 使用的 Agent Profile (可选, 默认主 Agent)
    pub agent_id: Option<String>,
    /// 指定模型 (可选)
    pub model: Option<String>,
}

pub enum MessagePart {
    Text { content: String },
    File { path: String, content: Option<String> },
    Command { command: String },  // 直接命令
}

// 响应: SSE 流
// event: assistant_delta
// data: {"token": "let", "turn_id": "turn-123"}
//
// event: tool_call
// data: {"tool_call_id": "tc-1", "name": "readFile", "args": {...}}
//
// event: tool_result
// data: {"tool_call_id": "tc-1", "output": "...", "success": true}
//
// event: done
// data: {"turn_id": "turn-123", "outcome": "completed"}
```

#### POST /api/v1/sessions/{id}/message/async (异步)

```rust
// 请求
pub struct AsyncMessageRequest {
    pub content: Vec<MessagePart>,
    pub agent_id: Option<String>,
    pub callback_url: Option<String>,  // 完成后回调
}

// 响应: 202 Accepted
pub struct AsyncResponse {
    pub turn_id: String,
    pub status: "accepted" | "running" | "completed" | "failed",
    pub status_url: String,  // GET /api/v1/sessions/{id}/turns/:turn_id
}
```

#### GET /api/v1/sessions/{id}/turns/:turn_id (查询异步任务状态)

```rust
// 响应
pub struct TurnStatus {
    pub turn_id: String,
    pub status: "running" | "completed" | "failed" | "aborted",
    pub events: Vec<TurnEvent>,     // 目前为止的事件
    pub started_at: DateTime,
    pub completed_at: Option<DateTime>,
}
```

#### POST /api/v1/agents/{id}/execute (创建 root execution，会走正常 AgentLoop)

这是 "Agent as API" 的核心端点。允许外部系统创建一个独立 session，
并异步启动对应的 root execution：

```rust
pub struct AgentExecuteRequest {
    /// Agent 任务描述
    pub task: String,
    /// 工作目录
    pub working_dir: Option<String>,
    /// 子会话上下文 override（可选）
    pub context_overrides: Option<SubagentContextOverrides>,
}

// 202 Accepted
pub struct AgentExecuteResponse {
    pub accepted: bool,
    pub session_id: String,
    pub turn_id: String,
    pub agent_id: String,
}
```

### 4.6 Shared Observability

父流程当前可以通过结构化生命周期事件和运行时指标消费子执行域结果，而不需要共享父可变状态：

- `SubRunFinished.result`
- `SubRunFinished.step_count`
- `SubRunFinished.estimated_tokens`
- runtime status 中的 `metrics.subrunExecution`

这部分是 Astrcode 当前允许扩展的共享面；父状态直写、权限提示直通、缓存共享仍不在本轮语义范围内。

### 5.3 WebSocket API

```
WS /api/v1/ws

// Client → Server
{
    "type": "subscribe",
    "session_id": "session-123"
}
{
    "type": "send_message",
    "session_id": "session-123",
    "content": "重构 auth 模块"
}
{
    "type": "abort",
    "session_id": "session-123"
}

// Server → Client
{
    "type": "event",
    "event": {
        "type": "assistant_delta",
        "token": "我会使用...",
        "turn_id": "turn-123"
    }
}
{
    "type": "error",
    "error": {"code": "TOOL_DENIED", "message": "工具被策略拒绝"}
}
```

### 5.4 认证与授权

```rust
// 简化的 API Key 认证
pub struct ApiConfig {
    /// API Key (环境变量 ASTRCODE_API_KEY)
    pub api_key: Option<String>,
    /// CORS 配置
    pub cors_origins: Vec<String>,
    /// 请求限流
    pub rate_limit: Option<RateLimitConfig>,
}

// Middleware 验证
async fn auth_middleware<B>(
    req: Request<B>,
    next: Next<B>,
) -> Result<Response> {
    if let Some(expected_key) = &config.api_key {
        if req.headers().get("Authorization") != Some(&expected_key.as_bytes()) {
            return unauthorized();
        }
    }
    next.run(req).await
}
```

---

## 7. 关键实现决策

### 7.1 子 Agent 事件存储策略

**决策**: 子 Agent 事件写入同一个 EventLog, 但标记 `parent_turn_id`

**理由**:
- 复用现有的 EventLog 和 EventTranslator
- 在 Event Stream 中可追溯完整的父子关系
- 不需要引入新的存储后端

**实现**:
```rust
// StorageEvent 扩展
pub enum StorageEvent {
    // ... 现有 variants
    
    /// 子 Agent Turn 开始
    SubAgentTurnStart {
        storage_seq: u64,
        sub_turn_id: String,
        parent_turn_id: String,
        agent_profile: String,
        task: String,
        timestamp: DateTime,
    },
    
    /// 子 Agent Turn 完成
    SubAgentTurnEnd {
        storage_seq: u64,
        sub_turn_id: String,
        parent_turn_id: String,
        outcome: String,
        summary: String,
        token_usage: Option<TokenUsage>,
        timestamp: DateTime,
    },
}
```

### 7.2 Tool Set 隔离策略

**决策**: 子 Agent 默认只读工具, 可通过 Profile 配置覆盖

**理由**:
- 安全性: 子 Agent 不应意外修改用户文件
- 可控性: 父 Agent 控制子 Agent 能力
- 参考 Codex: 只读 Agent (`explore`) 默认安全

### 7.3 模型选择策略

**决策**: 子 Agent 可使用独立模型 (可能更小/更快)

**理由**:
- 成本控制: 探索类任务不需要 GPT-4o
- 速度: 简单任务用 GPT-4o-mini
- 参考 Pi-Mono: 多 Provider, 灵活选择

**实现**:
```rust
/// 模型选择逻辑
fn resolve_model(profile: &AgentProfile, config: &LlmConfig) -> String {
    profile.model_preference
        .or(config.default_sub_agent_model)
        .unwrap_or(config.default_model)
}
```

### 7.4 取消传播策略

**决策**: 父 Agent 取消 → 所有活跃的子 Agent 也取消

**实现**:
```rust
// CancelToken 天然支持 (tokio_util::sync::CancellationToken)
let parent_cancel = CancelToken::new();
let child_cancel = parent_cancel.child();  // 子 token

// 父取消时, 子也自动取消
parent_cancel.cancel();  // child_cancel.is_cancelled() == true
```

---

## 8. 流式事件格式 (扩展)

### 8.1 新增 SSE 事件类型

```rust
// 现有的 StorageEvent 扩展 + 新事件

/// Agent Tool 调用 (从父 Agent 视角)
pub struct AgentToolCallEvent {
    pub name: String,           // Agent Profile ID
    pub task: String,
    pub max_steps: Option<usize>,
    pub token_budget: Option<usize>,
}

/// Agent 工具结果
pub struct AgentToolResultEvent {
    pub status: "completed" | "failed" | "aborted",
    pub summary: String,        // LLM 可读的摘要
    pub artifacts: Vec<ArtifactRef>,  // 产生的文件/变更
    pub token_usage: Option<TokenUsage>,
}

/// Agent 执行中的进度
pub struct AgentProgressEvent {
    pub sub_turn_id: String,
    pub step: usize,
    pub max_steps: usize,
    pub current_action: String,  // 当前正在做什么
}
```

### 8.2 前端事件投影

前端需要将子 Agent 事件投影为嵌套的展示:

```
用户消息: "重构 auth"
  → Agent 思考...
  → 📦 调用 Agent: explore 
     → 正在执行第 2/5 步... (progress)
     → 正在读取 auth.rs... (progress)
     → ✅ 完成 (summary: "auth 模块有3个函数...")
  → Agent 继续思考... (基于 explore 的结果)
  → 📦 调用 Agent: plan
     → ...
```

> **TODO**: 前端嵌套展示的具体交互细节需要询问用户确认

---

## 9. 安全与权限

### 9.1 策略引擎适配

子 Agent 的策略评估需要额外上下文:

```rust
pub struct SubAgentPolicyContext {
    /// 父 call 的 turn_id
    pub parent_turn_id: String,
    /// Agent Profile
    pub agent_profile: String,
    /// 子 Agent 是否允许请求用户审批 (通常不允许)
    pub can_request_approval: bool,
}

impl PolicyEngine for SubAgentPolicyEngine {
    async fn check(&self, call: &CapabilityCall, ctx: &PolicyContext) -> PolicyVerdict {
        // 1. 检查工具是否在 allowed_tools 中
        if !self.allowed_tools.contains(&call.tool_name) {
            return PolicyVerdict::deny("工具不在 Agent 允许列表中");
        }
        
        // 2. 子 Agent 不能请求用户审批 (无UI权限)
        if call.requires_approval {
            return PolicyVerdict::deny("子 Agent 不能请求用户审批");
        }
        
        // 3. 继承父策略引擎的判断
        self.parent.check(call, ctx).await
    }
}
```

### 9.2 API 安全

| 安全措施 | 实现 |
|----------|------|
| API Key | `Authorization: Bearer <key>` |
| CORS | 可配置白名单 |
| 限流 | 每 IP 每分钟请求数 |
| 请求体大小 | 限制最大 Prompt 长度 |
| 工具黑名单 | 通过策略引擎配置 |
| 文件系统权限 | 继承 OS 级权限 |

---

## 10. 配置系统扩展

### 10.1 Agent 配置 (Claude Markdown Frontmatter)

当前实现会按优先级加载这些目录：

- builtin agents
- `~/.claude/agents`
- `~/.astrcode/agents`
- `<working_dir>/.claude/agents`
- `<working_dir>/.astrcode/agents`

同名 agent 按后者覆盖前者，文件格式与 Claude Code sub-agents 一致：

```md
---
name: review
description: 审查代码的质量、安全性和最佳实践
tools: [readFile, grep]
disallowedTools: [shell]
model: quality
---
重点审查行为回归、边界条件和测试缺口，避免只给样式建议。
```

### 10.2 API 配置

```toml
[api]
enabled = true
host = "0.0.0.0"
port = 6543
api_key = "your-secret-key"  # 或环境变量 ASTRCODE_API_KEY

[api.cors]
origins = ["http://localhost:3000", "https://your-app.com"]

[api.rate_limit]
requests_per_minute = 60
burst = 10
```

---

## 11. 与竞品的差异化

| 特性 | Astrcode 方案 | 竞品 |
|------|---------------|------|
| Agent as Tool | ✅ 内置 Profile 系统, 策略安全 | Codex 有 spawn_agent, 但无 Profile 管理 |
| API 开放 | ✅ REST + SSE + WS 全支持 | OpenCode 只有 REST+SSE |
| 时间旅行 | ❌ (Phase 2) | Kimi-CLI 有 D-Mail, OpenCode 有 revert |
| 安全沙箱 | ❌ (Phase 3) | 仅 Codex 有完整沙箱 |
| 多模型路由 | ✅ 子 Agent 独立模型选择 | Pi-Mono 有, 但不在 Agent 级别 |
| 事件嵌套 | ✅ parent_turn_id 关联 | 无明确先例 |

---
