# AgentLoop 连进路线图

> 最后更新：2026-04-03
> 范围：`crates/runtime-agent-loop/src/agent_loop` 及相关模块

本文档定义 5 个演进阶段 + 1 个远期占位，每个阶段独立可交付、有明确验收标准.

## P1：状态机化 ✅ 已完成

将隐式终止改为显式 `TurnOutcome`，移除 `max_steps`.

**实现位置**:
- `crates/runtime-agent-loop/src/agent_loop.rs` — `TurnOutcome` 枚举 (Completed/Cancelled/Error)
- `StorageEvent::TurnDone.reason` 字段 (`"completed" | "cancelled" | "error"`)
- `finish_turn` / `finish_with_error` / `finish_interrupted` 统一返回 `Result<TurnOutcome>`

详见 [ADR-0006](../adr/0006-turn-outcome-state-machine.md).

## P2：并行工具执行 ✅ 已完成

独立工具调用可并发执行，按 `concurrency_safe` 分组，通过 `buffer_unordered(concurrency_limit)` 控制.

**实现位置**:
- `crates/runtime-agent-loop/src/agent_loop/tool_cycle.rs` — `execute_tool_calls` 并行执行管线
- `AgentLoop.max_tool_concurrency` — 并发度上限 (默认从 `runtime-config` 读取)
- 安全工具走 `buffer_unordered()`, 非安全工具串行执行

---

## P3：上下文压缩 + Token Budget ✅ 已完成

### 实现状态

| 子阶段 | 状态 | 实现位置 |
|--------|------|---------|
| P3.1 Token 估算 | ✅ | `crates/runtime-agent-loop/src/context_window/token_usage.rs` — `estimate_request_tokens()` |
| P3.2 百分比阈值 + Config | ✅ | AgentLoop 已持有 `auto_compact_enabled`, `compact_threshold_percent(默认90)`, `compact_keep_recent_turns`, `tool_result_max_bytes(默认100KB)` |
| P3.3 Tool Result Budget | ✅ | 工具结果截断已集成 (100KB 上限) |
| P3.4 Auto-Compact | ✅ | `crates/runtime-agent-loop/src/context_window/compaction.rs` — `auto_compact()`, 含 compact prompt、`<summary>` XML block 解析、prefix 递归降级重试 |
| P3.4b Micro-Compact | ✅ | `crates/runtime-agent-loop/src/context_window/microcompact.rs` — 微调/增量压缩 |
| P3.6 Token Budget / Auto-Continue | ✅ | `crates/runtime-agent-loop/src/agent_loop/token_budget.rs` — 已完整实现 Token 预算解析 、续命决策（nudge 消息注入）、diminishing-returns 检测 |

**Compaction 设计要点**:
- 保留最近 K 轮完整对话不动 (`compact_keep_recent_turns`)
- boundary 之前的消息调用 LLM 生成摘要，替换为一条 `UserMessage(CompactSummary)`
- 413 panic 时自动递归 drop 最老 turn group (最多 3 次)
- 保留已有 compact summary 到下一次 compaction input 中 (折叠历史不丢失)
- `RuntimeConfig` 新增: `default_token_budget`, `continuation_min_delta_tokens`, `max_continuations`
- `UserMessageOrigin::AutoContinueNudge | CompactSummary` 区分自动续命来源

### 验收标准

- [x] 每个 step 正确计算并暴露 token 用量 (`PromptMetrics` 事件)
- [x] token 用量超过 context window 的 90% 时自动触发 compact
- [x] 单个 tool result 超过 100KB 时自动截断
- [x] compact 后的消息数量显著减少
- [x] compact 后的对话能继续正常交互
- [x] `compact_threshold_percent` 可通过 config 自定义
- [x] P3.6: 用户消息包含 `+Nk` 时自动设定 token 预算并自动续命
- [x] Policy 决策点 `decide_context_strategy` 可触发 `Compact` / `Summarize` / `Truncate` / `Ignore`

---

## P4：错误恢复 ✅ 已完成

对可恢复的 API 错误进行自动重试，而非直接终止 turn.

### 实现状态

| 子阶段 | 状态 | 实现位置 |
|--------|------|---------|
| P4.1 413 → Reactive Compact | ✅ | `crates/runtime-agent-loop/src/agent_loop/turn_runner.rs` — `run_turn()` 内 413 错误时触发 `auto_compact()` 并重试，最多 3 次 |
| P4.2 Max Output Tokens → 重试 | ✅ | `crates/runtime-agent-loop/src/agent_loop/turn_runner.rs` — 检测 `finish_reason.is_max_tokens()` 注入 nudge 继续生成，最多 3 次 |
| P4.3 结构化 LlmError | ✅ | `crates/runtime-llm/src/lib.rs` — `LlmError` 枚举 + `FinishReason` 枚举；`classify_http_error()` 统一分类 |

### 验收标准

- [x] 模拟 413 错误时 turn 级别触发 reactive compact 并恢复
- [x] 模拟 max_output_tokens 截断时自动继续生成
- [x] 恢复失败时正确终止 turn

### 影响范围

- `crates/runtime-agent-loop/src/agent_loop/turn_runner.rs` (错误恢复逻辑)
- `crates/runtime-llm/` (结构化错误类型)

---

<!-- ## P5：API 韧性层 ✗ 未开始(暂时不做)

降低 API 调用成本 (prompt caching) 和提升可用性 (模型降级、速率限制处理).

### 需求

#### P5.1 Cache Breakpoints (Single-marker)

每次请求只在最后一条消息上放 `cache_control` marker.

#### P5.2 Model Fallback

主模型 529 连续 3 次 → 切换备选模型，剥离 thinking signatures.

#### P5.3 Rate Limit 处理

429 响应分级: (`retry_after < 30s` 重试) / (`>= 30s` fallback) / (无 fallback 指数退避).

### 验收标准

- [ ] 每次 API 请求自动在最后一条消息插入 cache marker
- [ ] 主模型 529 超过 3 次后自动切换备选模型
- [ ] 切换模型后 thinking signatures 被正确剥离
- [ ] 429 响应按 retry_after 时长分级处理

--- -->

## P6：子 Agent (远期占位) ✗ 未开始

递归 `AgentLoop`：`AgentTool` 创建并运行子 `AgentLoop`，天然支持三种协作模式：
- 委派 (单次调用)
- 并行团队 (多个 AgentTool 并发)
- 接替 (跨 turn 上下文传递)

详见 [agent-loop-roadmap.md 原始文档 P6 章节](#p6子-agent远期占位).

---

## 依赖关系

```
P1 (状态机化) ✅
  ├── P2 (并行工具执行) ✅
  └── P3 (上下文压缩 + Token Budget) ✅
  ├── P4 (错误恢复，依赖 P3 的 compaction) ✅
      └── P5 (API 韧性，可与 P4 并行) ✗

P6 (子 Agent，独立于 P2-P5，远期占位) ✗
```

# P7: Phase 9: ACP / MCP Entry Points — Not Started

---

## 远期 TODO

### Stop Hooks (后处理扩展点) — 中优先级

turn 结束后运行的后处理钩子框架 (用户自定义 hooks / prompt suggestion / memory extraction).

### Streaming Tool Execution (流式工具执行) — 低优先级

不等 LLM 响应完全接收就开始执行工具. 需 P2 完成后才能启动.

### Tool Use Summaries (工具摘要) — 低优先级

小模型生成 30 字工具摘要，用于非交互/SDK 消费者.
