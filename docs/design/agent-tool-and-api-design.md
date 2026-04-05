# Agent 系统现代化设计文档

## 概述

本文档基于对Codex、Kimi-CLI、OpenCode、Pi-Mono、claude-code-sourcemap 五个项目的深度分析，结合Astrcode现有架构，设计将 Agent Loop 作为 Tool 供 LLM 调用，并对外暴露完整 API 的解决方案。

---

## 1. 五大项目 Agent 系统深度分析

### 1.1 Codex (Rust/事件驱动)

#### 核心架构

**架构模式**: 事件驱动 + 编排器模式 (Orchestrator Pattern)

```
用户输入 → 构建 Turn 上下文 → LLM 流式调用 → Tool Orchestrator (审批→沙箱→执行) → 事件流 → 响应完成
```

**关键设计**:
- **双通道异步通信**: `tx_sub` 提交请求，`rx_event` 接收流式事件
- **Tool Orchestrator 三层模型**:
  - 审批层: `ApprovalStore` 缓存决策 → 避免重复询问用户
  - 沙箱层: 平台隔离 (macOS Seatbelt/Linux Landlock/Windows Job)
  - 执行层: 重试、网络审计、逃逸回退
- **事件持久化**: JSONL roll-out recorder, 支持 session resume/fork
- **多 Agent 协调**: `spawn_agent`, `send_message`, `wait_agent` 工具

**API 暴露**: 无外部 HTTP API，纯进程内通信。通过 TUI 或 CLI 提供用户界面。

**值得借鉴的设计**:
- ✅ 上下文自动压缩 (接近 token 上限时)
- ✅ Guardian 自动安全审查
- ✅ 网络请求审计 (拒绝→审批→执行全链路)

### 1.2 Kimi-CLI (Python/时间旅行)

#### 核心架构

**架构模式**: 单步迭代循环 + 时间旅行系统

```
KimiSoul._agent_loop() {
  1. 上下文检查 (token数+reserved < max_context)
  2. kosong.step() → LLM 调用
  3. KimiToolset.handle() → 工具执行
  4. 批准管制 (YOLO模式 vs 用户审批)
  5. D-Mail 时间旅行 (如果有待处理消息)
  6. 停止条件判断
}
```

**关键设计**:
- **D-Mail 时间旅行**: 允许 Agent "向过去传递消息"
  - `SendDMail` 工具 → 创建包含 checkpoint_id 和 message 的 DMail
  - 主循环每轮检查 `fetch_pending_dmail()`
  - 执行 `revert_to(checkpoint_id)` 回滚上下文
  - 用 DMail 内容继续执行 (模拟"先见之明")
- **上下文 JSONL 持久化**: 带 checkpoint 标记，支持时间旅行
- **Approval 系统**: 通过 `current_tool_call` ContextVar 传递审批上下文

**API 暴露**: 命令行 + IDE 集成 (ACP协议，WebSocket + JSON-RPC)
  - IDE 通过 WebSocket 订阅事件、发送请求
  - JSON-RPC 方法: `initialize`, `prompt/send`, `session/subscribe`

**值得借鉴的设计**:
- ✅ D-Mail 时间旅行 (unique feature)
- ✅ 上下文压缩 (SimpleCompaction)
- ✅ 多 Provider 支持 (Kimi/OpenAI/Anthropic/Gemini/Vertex)

### 1.3 OpenCode (TypeScript/HTTP API)

#### 核心架构

**架构模式**: 分层架构 + Hono HTTP Server + Vercel AI SDK

```
┌────── CLI / Desktop (Tauri) ──────┐
├────── Hono HTTP Server ───────────┤
│  ├── Session 路由                    │
│  ├── Config 路由                    │
│  ├── Permission 路由                │
│  └── MCP 路由                      │
├────── Session 管理层                │
│  ├── SessionPrompt (prompt执行)    │
│  ├── SessionProcessor (LLM循环)     │
│  └── Compaction/Revert             │
├────── LLM & Tool 执行层            │
│  ├── LLM.stream (Vercel AI SDK)    │
│  ├── Tool Registry & Resolution    │
│  └── Permission Enforcement        │
└────── Plugin & Provider 系统       │
```

**核心 API 端点**:

| 端点 | 描述 | 流式 |
|------|------|------|
| `POST /session/:sessionID/message` | 主消息端点，流式返回响应 | ✅ |
| `POST /session/:sessionID/prompt_async` | 异步执行 (204) | ❌ |
| `POST /session/:sessionID/command` | 直接执行命令 | ❌ |
| `POST /session/:sessionID/fork` | 在特定消息分叉 | ❌ |
| `POST /session/:sessionID/abort` | 取消进行中的执行 | ❌ |
| `POST /session/:sessionID/revert` | 撤销消息效果 | ❌ |
| `POST /session/:sessionID/summarize` | 压缩对话 | ❌ |
| `POST /session/:sessionID/share` | 创建分享链接 | ❌ |

**Agent 定义 (AGENTS.md)**:
```yaml
agents:
  custom-agent:
    name: "Custom Agent"
    mode: "primary" | "subagent" | "all"
    model: "openai/gpt-4"
    permission:
      "*": "allow"
      question: "deny"
      read: "ask"
```

**工具系统**:
- TypeBox schema 验证
- 执行上下文包含: `sessionID`, `agent`, `abort`, `messages`, `ask()`, `metadata()`
- 权限模式: `allow` / `ask` / `deny` (glob 模式匹配)
- 工具输出自动截断 + 存储到文件

**值得借鉴的设计**:
- ✅ 完整的 HTTP JSON 流式 API
- ✅ Agent 定义与权限配置系统
- ✅ Session fork/abort/summarize 操作
- ✅ Instance-based 多工作空间隔离

### 1.4 Pi-Mono (TypeScript/多模式)

#### 核心架构

**架构模式**: 事件流状态机 + 多模式架构

```
┌─ @mariozechner/pi-ai (统一LLM API, 17+ Provider)
    ↓
┌─ @mariozechner/pi-agent-core (状态化 Agent Runtime)
    ↓
┌─ @mariozechner/pi-coding-agent (CLI, RPC, 交互模式)
    ↓
┌─ 表现层: TUI / Web UI / Slack Bot
```

**Agent Loop 事件流**:
```
agent_start
 └─ turn_start
     ├─ message_start (user prompt)
     ├─ message_end
     ├─ message_start (assistant response)
     ├─ message_update (流式 chunks) ← LLM 流式输出
     ├─ message_end (complete)
     │
     └─ [若调用工具]
         ├─ tool_execution_start
         ├─ tool_execution_update (partial results)
         ├─ tool_execution_end
         ├─ message_start (tool result)
         └─ message_end
     └─ turn_end
 └─ agent_end
```

**多模式服务**:
- **互动模式**: 完整 TUI
- **打印模式**: 纯 stdout JSON/文本
- **RPC 模式**: JSON-line 协议在 stdin/stdout
- **SDK 模式**: 程序化导入

**RPC 协议**:
```typescript
type RpcCommand =
  | { type: 'prompt', message: string, context?: Context }
  | { type: 'command', id: string, args: string[] }
  | { type: 'abort' }
  | { type: 'exit' };

type RpcEvent =
  | { type: 'event', event: AgentEvent }
  | { type: 'response', id: string, text: string }
  | { type: 'error', message: string };
```

**值得借鉴的设计**:
- ✅ 明确定义的事件状态机，UI 解耦
- ✅ SDK/RPC 双模式暴露 (适合集成)
- ✅ Tool hooks (beforeToolCall 拦截, afterToolCall 修改)
- ✅ 多 Provider 抽象层 (单一接口, 17+ Provider)

### 1.5 Claude-Code-Sourcemap (文档/分析)

#### 核心架构

这是 Claude Code 的源码地图文档项目，提供 Anthropic Claude Code 的内部架构分析。

**关键架构发现**:
- Claude Code 使用 **MCP (Model Context Protocol) Server** 暴露工具能力
- Agent 循环: `prompt → LLM → tool_calls → execute → results → 循环`
- 工具执行通过 MCP 协议:
  - `tools/list` - 列出可用工具
  - `tools/call` - 执行工具调用
- 沙箱执行: macOS Seatbelt / Linux seccomp-bpf

**值得借鉴的设计**:
- ✅ MCP 协议作为工具标准接口
- ✅ 清晰的源码文档化方式 (sourcemap)

### 1.6 横向对比总结

| 特性 | Codex | Kimi-CLI | OpenCode | Pi-Mono | Astrcode (现有) |
|------|-------|----------|----------|---------|-----------------|
| **语言** | Rust | Python | TypeScript | TypeScript | Rust |
| **Agent 暴露方式** | 进程内 | CLI/ACP | HTTP API | RPC/SDK | HTTP/SSE |
| **事件系统** | 异步 Channel | 同步循环 | SSE 流式 | 事件流 | StorageEvent |
| **工具系统** | Handler Trait | Pydantic 类 | TypeBox | TypeBox | Tool Trait |
| **审批系统** | ApprovalStore | Approval Runtime | Permission 匹配 | Hooks | Policy Engine |
| **上下文管理** | 自动压缩 | JSONL + D-Mail | Compaction | Context 转换 | Compaction |
| **多 Agent** | ✅ spawn_agent | ❌ | ❌ | ❌ | ❌ |
| **时间旅行** | ❌ | ✅ D-Mail | ✅ revert | ❌ | ❌ |
| **安全沙箱** | ✅ | ❌ | ❌ | ❌ | ❌ |

---

## 2. Astrcode 现有架构分析

### 2.1 现有架构概览

```
Layer 1: protocol + core (纯 DTO + 契约)
    ↓
Layer 2: runtime-tool-loader / runtime-config / runtime-llm / runtime-prompt
    ↓
Layer 3: runtime-agent-loop (AgentLoop 执行引擎)
    ↓
Layer 4: runtime (RuntimeService 门面)
    ↓
Layer 5: server (HTTP/SSE) + plugin → src-tauri (桌面壳)
```

### 2.2 现有 Agent Loop 状态机

```
Turn Execution (turn_runner.rs):
1. build_bundle()       → 上下文构建
2. build_plan()          → Prompt 组装
3. build_step_request()  → 请求装配
4. maybe_compact()       → 按需压缩
5. generate_response()   → LLM 调用 (llm_cycle.rs)
6. process_tool_calls()  → 工具执行 (tool_cycle.rs)
   ↓ 回到步骤1 如果还有工具调用
   或 TurnCompleted
```

### 2.3 现有 API 能力

- `POST /sessions` - 创建会话
- `POST /sessions/:id/prompt` - 发送消息, SSE 流式
- `GET /sessions/:id/events` - 订阅事件流
- `GET /sessions` - 列出会话

### 2.4 问题与不足 (与竞品对比)

| 差距 | 影响 | 竞品做法 |
|------|------|----------|
| Agent 不能作为 Tool 被 LLM 调用 | 无法实现子任务委派 | Codex spawn_agent, OpenCode SubtaskPart, Pi-Mono task tool |
| API 不够开放 | 第三方难以集成 | OpenCode 完整 REST API, Pi-Mono RPC/SDK |
| 缺少异步/批量执行 | 只能单用户交互 | OpenCode prompt_async, Codex fire-and-forget |
| 工具粒度固定 | 无法灵活组合 | Codex Orchestrator, OpenCode 权限过滤 |
| 无时间旅行/上下文恢复 | 调试困难 | Kimi-CLI D-Mail, OpenCode revert |

---

## 3. 设计目标

### 3.1 核心目标

1. **Agent as Tool**: 将 Agent Loop 暴露为 Tool，允许 LLM 调用子 Agent 完成任务
2. **开放 API**: 提供完整的 REST + WebSocket API, 支持第三方集成
3. **安全可控**: 保持现有的策略引擎 + 审批系统
4. **向后兼容**: 不破坏现有功能

### 3.2 设计原则

- **最小侵入性**: 尽可能复用现有架构,只添加新的边界层
- **协议一致性**: 遵循现有 protocol/core 的 DTO 映射模式
- **编译隔离**: 新 crate 只依赖 runtime, 不直接修改 core/agent-loop
- **事件驱动**: 复用现有的 StorageEvent 系统

---

## 4. 方案一: Agent as Tool (子代理系统)

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
}
```

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

## 5. 方案二: 开放 API 设计

### 5.1 API 概览

基于 OpenCode 和 Pi-Mono 的成熟 API 设计, 扩展 Astrcode 的 server crate:

```
POST /api/v1/sessions              - 创建会话
GET  /api/v1/sessions              - 列出会话
GET  /api/v1/sessions/:id          - 获取会话详情
DELETE /api/v1/sessions/:id        - 删除会话

POST /api/v1/sessions/:id/message         - 发送消息 (流式)
POST /api/v1/sessions/:id/message/async   - 发送消息 (异步)
GET  /api/v1/sessions/:id/messages        - 获取消息历史
POST /api/v1/sessions/:id/abort           - 中止执行

POST /api/v1/sessions/:id/fork            - 分叉会话
POST /api/v1/sessions/:id/summarize       - 压缩会话
POST /api/v1/sessions/:id/revert          - 撤销到指定位置

GET  /api/v1/sessions/:id/events          - 订阅事件 (SSE)
GET  /api/v1/sessions/:id/events/stream   - WebSocket 事件流

GET  /api/v1/agents                 - 列出可用 Agent
POST /api/v1/agents/:id/execute     - 直接执行 Agent 任务

GET  /api/v1/tools                  - 列出可用工具
POST /api/v1/tools/:id/execute      - 执行单个工具

GET  /health                        - 健康检查
```

### 5.2 核心端点详情

#### POST /api/v1/sessions/:id/message (流式)

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

#### POST /api/v1/sessions/:id/message/async (异步)

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
    pub status_url: String,  // GET /api/v1/sessions/:id/turns/:turn_id
}
```

#### GET /api/v1/sessions/:id/turns/:turn_id (查询异步任务状态)

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

#### POST /api/v1/agents/:id/execute (直接执行 Agent, 不调用主循环)

这是 "Agent as API" 的核心端点。允许外部系统直接调用预定义的 Agent:

```rust
pub struct AgentExecuteRequest {
    /// Agent 任务描述
    pub task: String,
    /// 工作目录
    pub working_dir: Option<String>,
    /// 额外参数
    pub options: Option<AgentOptions>,
}

// SSE 流式响应
// event: agent_event
// data: {"type": "tool_call", ...}
//
// event: done
// data: {"summary": "...", "artifacts": [...]}
```

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

## 6. Crate 组织方案

### 6.1 新增/修改的 Crate

```
crates/
  ├── runtime-agent-loop/    (现有, 修改: 支持子 Agent 调度)
  ├── runtime-agent-api/     (新增: 扩展 REST API + WebSocket)
  ├── runtime-agent-profiles/(新增: 预置 Agent Profile 定义)
  └── runtime-agent-tool/    (新增: runAgent Tool 实现)
```

### 6.2 runtime-agent-profiles

```
crates/runtime-agent-profiles/
  ├── Cargo.toml
  └── src/
      ├── lib.rs                    // Profile 注册表
      ├── profiles/
      │   ├── explore.rs            // 代码探索 Agent
      │   ├── plan.rs               // 任务规划 Agent
      │   ├── execute.rs            // 代码执行 Agent
      │   └── review.rs             // 代码审查 Agent
      └── serde.rs                  // 从配置文件加载 Profile
```

### 6.3 runtime-agent-tool

```
crates/runtime-agent-tool/
  ├── Cargo.toml
  └── src/
      ├── lib.rs                    // 导出 RunAgentTool
      ├── tool.rs                   // Tool trait 实现
      ├── sub_agent_loop.rs         // 子 Agent 执行引擎
      └── result_summary.rs         // 结果摘要/截断
```

### 6.4 runtime-agent-api

```
crates/runtime-agent-api/
  ├── Cargo.toml
  └── src/
      ├── lib.rs                    // API 路由组装
      ├── routes/
      │   ├── sessions.rs           // 会话操作
      │   ├── messages.rs           // 消息操作
      │   ├── agents.rs             // Agent 操作
      │   ├── tools.rs              // 工具操作
      │   └── events.rs             // 事件订阅 (SSE/WS)
      ├── middleware/
      │   ├── auth.rs               // 认证
      │   ├── cors.rs               // CORS
      │   └── rate_limit.rs         // 限流
      ├── ws/
      │   ├── handler.rs            // WebSocket 处理器
      │   └── protocol.rs           // WS 消息协议
      └── openapi.rs                // OpenAPI/Swagger 文档
```

### 6.5 依赖关系

```
runtime-agent-profiles    → core, protocol
runtime-agent-tool        → runtime-agent-loop, runtime-agent-profiles
runtime-agent-api         → server, runtime, runtime-agent-tool, runtime-agent-profiles
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

TODO:细节需要询问用户
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

### 10.1 Agent Profiles 配置 (TOML)

```toml
# astrcode.toml 或 .astrcode/config.toml

[agents.explore]
name = "代码探索"
description = "读取和理解代码, 用于分析代码库时使用"
max_steps = 5
allowed_tools = ["readFile", "listDir", "findFiles", "grep"]
model_preference = "fast"

[agents.plan]
name = "任务规划"
description = "分析需求并制定执行计划, 不修改代码"
max_steps = 3
allowed_tools = ["readFile", "grep"]
model_preference = "balanced"

[agents.execute]
name = "代码执行"
description = "执行具体的代码变更任务"
max_steps = 10
allowed_tools = ["readFile", "writeFile", "editFile", "shell"]
model_preference = "quality"

[agents.review]
name = "代码审查"
description = "审查代码的质量、安全性和最佳实践"
max_steps = 5
allowed_tools = ["readFile", "grep"]
model_preference = "quality"

# 默认子 Agent 模型
[sub_agent]
default_model = "fast"
token_budget = 8000
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

## 12. 未来扩展 (Phase 2+)

### 12.1 D-Mail 时间旅行 (参考 Kimi-CLI)

```rust
pub struct DMail {
    pub message: String,
    pub checkpoint_id: usize,
}
```

### 12.2 沙箱执行 (参考 Codex)

macOS: Seatbelt / Linux: Landlock / Windows: Job Objects

### 12.3 自动 Agent 编排

LLM 自行决定创建/配置子 Agent (不仅是预置 Profile)

---

## 13. 风险与缓解

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| 子 Agent 无限递归 | Token 耗尽 → 费用爆炸 | 最大递归深度 (默认 3) |
| 子 Agent Token 失控 | 预算超支 | 强制 token_budget 参数 |
| 并发子 Agent 竞争 | 工具调用冲突 | 父 Agent 串行调用子 Agent |
| API Key 泄露 | 未授权访问 | 密钥轮换 + IP 白名单 |
| Prompt 注入 (子 Agent) | 安全漏洞 | 子 Agent 无审批权限 |
