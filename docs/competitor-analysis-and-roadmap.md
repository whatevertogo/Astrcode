# Coding Agent 竞品对比与 Astrcode 下一步建议

> 基于 Claude Code、Codex、OpenCode、KimiCLI、pi-mono 五个项目的源码分析，对比 Astrcode 现状。

---

## 1. 竞品特性矩阵

| 特性领域 | Claude Code | Codex | OpenCode | KimiCLI | pi-mono | Astrcode 现状 |
|---------|------------|-------|----------|---------|---------|-------------|
| **语言/运行时** | TypeScript/Node | Rust + TS | TypeScript/Bun | Python | TypeScript/Bun | Rust + Tauri |
| **工具数量** | 40+ | ~20 | ~20 | ~15 | ~15 | ~10 (内置+MCP) |
| **子代理** | Swarm 协调器模式 | Spawn/Wait/Send v1+v2 | Task 工具 | 劳动力市场系统 | 扩展实现 | AgentTree 多级树 |
| **LSP 集成** | 有 (单工具) | 无 | 一等公民，多语言预配置 | 无 | 无 | **无** |
| **沙箱/安全** | 权限模式 | 三层沙箱 + Guardian AI | 权限系统 (per-tool/agent/glob) | 无 | 无 | Policy Engine (策略模式) |
| **上下文压缩** | 4 级 (full/micro/snip/reactive) | 自动+手动 compaction | 自动 compaction | 自动 compaction (85%阈值) | 自动+手动 compaction | 基础 compaction |
| **记忆系统** | MEMORY.md + Auto-Dream + 会话记忆 | 两阶段流水线 (提取→合并) | 无 | AGENTS.md 层级 | MEMORY.md | 基础 (文件级) |
| **Hooks** | 20+ 生命周期事件 | 无 | 插件生命周期 hooks | 7+ 事件 (可阻塞/注入) | beforeToolCall/afterToolCall | **无** |
| **MCP** | 客户端 (stdio/SSE/OAuth) | 客户端 + 服务端 | 客户端 | 客户端 (stdio/HTTP/OAuth) | 明确拒绝 MCP，用 CLI 工具替代 | 客户端 (adapter-mcp) |
| **ACP 协议** | 无 | 无 | 有 (Zed/JetBrains) | 有 (Zed/JetBrains) | RPC 模式 | **无** |
| **Git Worktree** | 有 (工具级) | 无 | 有 (任务级隔离) | 无 | 无 | **无** |
| **会话分叉** | 无 | 无 | 从任意消息分叉 | Checkpoint + D-Mail 回溯 | 会话树 (JSONL 父指针) | Turn 级分支 (部分) |
| **扩展/插件** | 插件 + 市场 | MCP + Codex Apps | 插件 (生命周期 hooks) | 插件 (目录加载) | 扩展系统 + 包管理器 | 插件框架 (部分) |
| **多 LLM** | 仅 Anthropic | 仅 OpenAI | 20+ 提供商 | 多提供商 | 20+ 提供商 + 跨提供者切换 | Anthropic + OpenAI |
| **SDK/API** | 有 (SDK 模式) | codex exec (CI/CD) | REST API + SSE | Wire 协议 (多前端) | 4 种运行模式 | sdk crate (极简) |
| **计划模式** | Plan 工具 | Plan 工具 | Plan 工具 | 只读研究→计划→自动批准 | 无 | **无** |
| **语音输入** | 有 (STT) | 无 | 无 | 无 | 无 | 无 |
| **Cron 调度** | 有 (AGENT_TRIGGERS) | 无 | 无 | 后台任务 + 心跳 | 自管理调度事件 | 无 |

---

## 2. Astrcode 差距分析

### 核心短板 (对用户体验影响最大)

1. **上下文管理不够精细** — 只有基础 compaction，缺少 micro-compact（轻量清除旧工具结果）和多级策略。Claude Code 的 4 级压缩是长会话的关键。
2. **无 Hooks 系统** — 用户无法在工具调用前后、会话开始/结束等节点插入自定义逻辑。这是生态扩展的基础。
3. **SDK/API 不成熟** — sdk crate 几乎为空。无法被外部程序集成或用于 CI/CD 场景。
4. **无 LSP 集成** — OpenCode 凭借一等公民 LSP 在代码理解上有巨大优势。Astrcode 作为 Rust 项目，接入 rust-analyzer 等是天然优势。
5. **会话分叉不完整** — 其他项目都支持从任意点分叉/回溯会话，Astrcode 只有 turn 级分支。

### 差异化机会 (别人做得少，Astrcode 可以做得好的)

1. **Guardian AI 审查** — Codex 独有，用 LLM 做二次风险评估。Astrcode 的 Policy Engine 已经有策略模式基础，可以增强为 AI 驱动的安全层。
2. **ACP (Agent Client Protocol)** — OpenCode 和 KimiCLI 都支持 IDE 集成协议。Astrcode 作为桌面应用天然适合。
3. **MCP 服务端模式** — Codex 可以作为 MCP 服务端让其他代理调用。Astrcode 的 adapter-mcp 基础可以扩展双向能力。
4. **跨提供者会话切换** — pi-mono 支持在对话中途切换模型并保留上下文。这在多模型时代很有价值。

---

## 3. 下一步建议 (优先级排序)

### P0 — 基础体验完善 (1-2 周)

#### 3.1 多级上下文压缩策略

借鉴 Claude Code 的分级思路：

- **Micro-compact**: 替换旧工具结果为占位符 `[旧工具结果已清除]`，成本极低
- **Full compact**: LLM 总结历史对话，保留关键代码片段和错误信息
- **Budget-aware 触发**: 基于 token 用量自动选择压缩级别

实现位置: `session-runtime` 的 turn 执行循环中，在 compaction 触发点分级处理。

```
触发条件: token_usage > context_window - buffer
├─ 轻度超限 → micro-compact (清除旧工具结果)
├─ 中度超限 → micro + 截断早期历史
└─ 严重超限 → full compact (LLM 总结)
```

#### 3.2 Hooks 系统

定义生命周期事件和 hook 注册机制：

```
事件: PreToolUse, PostToolUse, SessionStart, SessionEnd,
      PreCompact, PostCompact, UserPromptSubmit, Stop

Hook 类型:
├─ Shell hook — 执行 shell 命令，可通过 exit code 阻止
├─ Transform hook — 修改输入/输出内容
└─ Notification hook — 异步通知（fire-and-forget）
```

实现位置: `core` 定义事件 trait，`kernel` 提供 hook 注册和分发，`session-runtime` 在关键节点触发。

### P1 — 生态扩展 (2-4 周)

#### 3.3 LSP 集成

参考 OpenCode 的设计，但利用 Rust 的优势：

- 定义 `LspClient` port trait (在 `core`)
- 实现 rust-analyzer, typescript-language-server, gopls 等适配器
- 暴露为工具: `lsp_diagnostics`, `lsp_hover`, `lsp_goto_definition`, `lsp_references`
- 工具级自动管理 LSP server 生命周期

实现位置: 新增 `adapter-lsp` crate，工具注册到 `adapter-tools`。

#### 3.4 SDK 成熟化

让 Astrcode 可嵌入：

```rust
// 目标 API 示例
let client = AstrcodeClient::connect("http://localhost:3000").await?;
let session = client.create_session(config).await?;
let mut stream = session.query("帮我重构这个函数").await?;
while let Some(event) = stream.next().await {
    // 处理流式事件
}
```

实现位置: `sdk` crate，基于 `protocol` 的 DTO 定义客户端。

#### 3.5 ACP (Agent Client Protocol)

支持 IDE 集成 (Zed, JetBrains, VS Code)：

- JSON-RPC over stdio 协议实现
- 注册为 Zed 等 IDE 的 agent 后端
- 复用现有 SSE 事件流，增加 stdio 传输层

实现位置: 新增 `server` 的 ACP 端点或独立 `adapter-acp` crate。

### P2 — 高级特性 (4-8 周)

#### 3.6 AI Guardian 安全层

在 Policy Engine 基础上增加 LLM 驱动的风险评估：

```
工具调用 → Policy Engine (规则匹配)
         → Guardian Agent (LLM 评估高风险操作)
            ├─ risk_score < 50 → 自动通过
            ├─ 50-80 → 提示用户确认
            └─ >= 80 → 自动拒绝
```

#### 3.7 MCP 双向模式

当前只有客户端。扩展为同时支持服务端，让其他 agent 可以调用 Astrcode 的能力。

#### 3.8 会话完整分叉

支持从任意消息点创建分支会话，独立发展。基于现有 EventStore 的事件溯源能力，自然适合。

---

## 4. Astrcode 的独特优势

不要只看差距，Astrcode 也有自己的优势：

| 优势 | 说明 |
|-----|------|
| **Rust 性能** | 唯一全 Rust 后端 (Codex 有 Rust 版但非主力)，启动快、内存少、无 GC |
| **Tauri 桌面应用** | 唯一原生桌面 GUI，不是纯 CLI/TUI |
| **六边形架构** | 最严格的端口-适配器分离，内核最干净 |
| **事件溯源** | EventStore + Projection 模式，天然适合会话回溯和分叉 |
| **Plugin 进程隔离** | 插件独立进程 + supervisor 管理，安全性最好 |
| **Prompt Assembly** | 分层 builder + cache-aware blocks，prompt 工程最精细 |

---

## 5. 推荐路线图

```
Phase 1 (现在)          Phase 2 (1-2 月)         Phase 3 (3+ 月)
┌──────────────┐     ┌──────────────┐     ┌──────────────┐
│ 多级压缩     │     │ LSP 集成     │     │ AI Guardian  │
│ Hooks 系统   │     │ SDK 成熟化   │     │ MCP 服务端   │
│ 计划模式     │     │ ACP 协议     │     │ 多模型切换   │
└──────────────┘     │ 会话分叉     │     │ 语音输入     │
                     └──────────────┘     └──────────────┘
```

**核心理念**: 先夯实基础体验（压缩、hooks、计划模式），再扩展生态（LSP、SDK、ACP），最后做高级特性（AI 审查、双向 MCP）。
