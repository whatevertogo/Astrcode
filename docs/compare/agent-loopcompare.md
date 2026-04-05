# Agent 系统现代化设计文档

## 概述

本文档基于对Codex、Kimi-CLI、OpenCode、Pi-Mono、claude-code-sourcemap 五个项目的深度分析，结合Astrcode现有架构，设计将 Agent Loop 作为 Tool 供 LLM 调用，并对外暴露完整 API 的解决方案。

---

## 1. 五大项目 Agent 系统深度分析

### 1.1 Codex Agent 系统 (Rust/事件驱动)

#### Agent Loop 设计

**核心循环**:
```rust
// 简化的 Agent Loop
async fn agent_loop(
    mut session: Session,
    llm_client: &LlmClient,
    tool_registry: &ToolRegistry,
    event_tx: &EventSender,
) {
    loop {
        // 1. 构建上下文 (messages + tool definitions)
        let context = session.build_context();
        
        // 2. 检查 token 预算，触发压缩
        if context.tokens_near_limit() {
            session.compact(llm_client).await?;
        }
        
        // 3. LLM 调用
        let (response, tool_calls) = llm_client.stream_completion(context).await?;
        session.add_assistant_message(response);
        
        // 4. 如果没有工具调用，结束循环
        if tool_calls.is_empty() {
            event_tx.send(TurnCompleted { outcome: "completed" });
            break;
        }
        
        // 5. Tool Orchestrator: 审批 → 沙箱 → 执行
        for tool_call in tool_calls {
            event_tx.send(ToolCallStarted { ... });
            
            // 5a. 检查 ApprovalStore (缓存的用户决策)
            let decision = approval_store.check(&tool_call);
            if decision == Pending {
                event_tx.send(ApprovalRequired { tool_call: &tool_call });
                let user_decision = wait_for_user_approval().await;
                approval_store.cache(&tool_call, &user_decision);
            }
            
            // 5b. 沙箱执行 (平台相关)
            let result = sandboxed_execute(&tool_call, platform_sandbox_config).await?;
            session.add_tool_result(tool_call.id, result);
            event_tx.send(ToolCallCompleted { ... });
        }
        
        // 6. 继续下一次 LLM 调用 (将工具结果加入上下文)
    }
}
```

**关键 Agent 设计**:
- **Tool Orchestrator**: 工具执行不是直接调用，而是经过三层管道:
  - `ApprovalLayer` → 用户决策缓存, 避免重复询问
  - `SandboxLayer` → 平台隔离执行 (macOS: Seatbelt, Linux: Landlock, Windows: Job Objects)
  - `ExecutionLayer` → 重试、超时、结果截断
- **spawn_agent 工具**: 允许 LLM 创建子 Agent
  - `spawn_agent(task: String, scope: String)` → 创建隔离的探索型子任务
  - `send_message(agent_id, message)` → 向子 Agent 发送指令
  - `wait_agent(agent_id)` → 等待子 Agent 完成
- **Guardian 安全层**: 自动检测危险操作 (网络请求、文件删除) 并强制审批
- **上下文压缩**: 接近 token 上限时自动总结历史, 保留关键信息
- **事件持久化**: JSONL 格式的 roll-out recorder, 支持 session resume/fork

**Agent 安全模型**:
```
用户请求 → LLM → Tool Call → Guardian 审查
                                  ├─ 安全 → 直接执行
                                  ├─ 可疑 → 检查 ApprovalStore
                                  │         ├─ 已有决策 → 执行
                                  │         └─ 无决策 → 询问用户
                                  └─ 危险 → 拒绝 (如 rm -rf /)
```

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

### 1.5 Claude Code (通过 claude-code-sourcemap 分析)

#### 核心架构

这是 Claude Code 的源码地图文档项目，提供 Anthropic Claude Code 的内部架构分析。


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
- `POST /sessions/{id}/prompt` - 发送消息, SSE 流式
- `GET /sessions/{id}/events` - 订阅事件流
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
