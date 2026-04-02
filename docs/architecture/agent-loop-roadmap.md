# AgentLoop 演进路线图

> 状态：草案
> 创建：2026-04-01
> 范围：`crates/runtime/src/agent_loop` 及相关模块


本文档定义 5 个演进阶段 + 1 个远期占位 + 3 个远期 TODO，每个阶段独立可交付、有明确验收标准。

## P1：状态机化(已经完成)

### 目标

将隐式终止改为显式 `TurnOutcome`，移除 `max_steps`，以用户中断和上下文压缩作为真正的安全网。(已经完成)

## P2：并行工具执行

### 目标

独立的工具调用可以并发执行，缩短多工具场景的总耗时。

### 需求

#### P2.1 `CapabilityDescriptor` 新增并发安全标记(已经完成)
---

## P3：上下文压缩管线

### 目标

支持长会话不超出 context window，分两层：轻量截断 + 重量级摘要。

### 需求

#### P3.1 每次 Step 计算 Token 用量

在 `turn_runner` 的 step 循环中，组装 `request_messages` 之后、调用 LLM 之前：

- 对当前 `request_messages` 做 token 估算
- 简单启发式：、使用 `tiktoken-rs` 做精确计数
- 将 token 数存入 `TurnState`，供后续 compact 决策使用
- 通过 `StorageEvent` 或日志暴露当前 token 用量（可观测性）

```rust
fn estimate_tokens(messages: &[LlmMessage]) -> usize
```

#### P3.2 百分比阈值 + Config 配置

Compact 触发条件不是绝对 token 数，而是 **占当前模型 context window 的百分比**：

```toml
# config.toml
[prompt]
# 当 token 用量占 context window 的百分比超过此值时触发 auto-compact
compact_threshold_percent = 90

# 单个 tool result 的最大字节数（超过则截断）
tool_result_max_bytes = 100_000
```

- `compact_threshold_percent` 默认 `90`，即用到 90% context window 时触发 compact
- 模型的 `context_window` 大小从 provider metadata 获取（不同模型不同）
- 阈值 × window 大小 = 实际触发 token 数

```rust
fn should_compact(estimated_tokens: usize, context_window: usize, threshold_percent: u8) -> bool {
    estimated_tokens >= context_window * threshold_percent as usize / 100
}
```

#### P3.3 Tool Result Budget（轻量）

在组装 `request_messages` 之前：

- 每个 `LlmMessage::Tool` 的 content 超过 `tool_result_max_bytes`（默认 100KB）时截断
- 截断后替换 content 为 `[truncated: original N bytes, showing first M bytes]`
- 保留原始 content 的前 N 字节供 LLM 理解上下文

```rust
fn apply_tool_result_budget(
    messages: &mut [LlmMessage],
    max_bytes: usize,
) -> usize  // 返回截断的工具结果数量
```

#### P3.4 Auto-Compact（重量）

当 `should_compact()` 返回 `true` 时：

1. 确定一个 **compact boundary**（保留最近 K 轮对话不动）
2. 对 boundary 之前的消息调用 LLM 生成摘要
3. 用一条 `System` 消息替换 boundary 之前的全部消息：

```
[Auto-compact summary]
- 用户曾要求修改 agent_loop.rs
- 已读取了 crates/runtime/src/agent_loop.rs
- 执行了 grep 搜索 "run_turn"
- ...
```

4. compact boundary 之后的消息保持原样

```rust
async fn auto_compact(
    messages: Vec<LlmMessage>,
    system_prompt: &str,
    provider: &dyn LlmProvider,
    config: &CompactConfig,
) -> Result<CompactResult>

struct CompactResult {
    messages: Vec<LlmMessage>,
    messages_removed: usize,
    tokens_freed: usize,
}
```

#### P3.5 Compact 元数据

compact 执行本身需要记录元数据：

- compact 发生的时间、截断的消息数、释放的 token 数
- 通过 `StorageEvent::CompactDone` 或日志暴露

#### P3.6 Token Budget / Auto-Continue

在 P3.1 的 token 计数基础上，支持用户设定 token 预算，让 agent 持续自主工作直到预算耗尽。

**预算声明**：用户在消息中附加预算标记，如 `"+500k"` 或 `"use 2M tokens"`：

```rust
fn parse_token_budget(user_message: &str) -> Option<u64>
// "+500k" → 500_000
// "2M" → 2_000_000
```

**延续逻辑**（在 `turn_runner` 的 turn 结束时）：

```rust
fn check_token_budget(
    turn_tokens_used: usize,
    budget: u64,
    continuation_count: usize,
    last_delta_tokens: usize,
) -> TokenBudgetDecision {
    // 用量 < 90% 预算 → 继续
    if (turn_tokens_used as f64) < budget as f64 * 0.9 {
        return TokenBudgetDecision::Continue { nudge: true };
    }
    // 连续 3 次续命且每次产出 < 500 token → 边际收益递减，停止
    if continuation_count >= 3 && last_delta_tokens < 500 {
        return TokenBudgetDecision::DiminishingReturns;
    }
    TokenBudgetDecision::Stop
}
```

**Nudge 消息**：当决定继续时，注入一条 user message 让 LLM 不停下来总结：

```
Stopped at {pct}% of token target ({turnTokens} / {budget}). Keep working -- do not summarize.
```

关键点：**明确告诉 LLM "do not summarize"**，否则 LLM 的默认行为是停止时生成摘要。

**Config 配置**：

```toml
[prompt]
# 单次 turn 的默认 token 预算（0 = 不限制，由 LLM 自然终止）
default_token_budget = 0

# 自动续命的 diminishing-returns 检测阈值
continuation_min_delta_tokens = 500
max_continuations = 3
```

### 验收标准

- [ ] 每个 step 正确计算并暴露 token 用量
- [ ] token 用量超过 context window 的 90% 时自动触发 compact
- [ ] 单个 tool result 超过 100KB 时自动截断
- [ ] compact 后的消息数量显著减少
- [ ] compact 后的对话能继续正常交互（不丢失关键上下文）
- [ ] 手动触发 compact 的 API 可用（如 `/compact` 命令）
- [ ] `compact_threshold_percent` 可通过 config 自定义
- [ ] 用户消息包含 `+Nk` 时自动设定 token 预算
- [ ] 预算未耗尽时 agent 自动续命（注入 nudge 消息）
- [ ] 连续 3 次续命且每次产出 < 500 token 时自动停止

### 影响范围

- `crates/runtime/src/agent_loop/turn_runner.rs`（token 估算 + 集成压缩管线 + budget 检查）
- `crates/runtime/src/agent_loop/compaction.rs`（新模块）
- `crates/runtime/src/agent_loop/token_budget.rs`（新模块：预算解析 + 续命决策）
- `crates/core/src/action.rs`（可能需要 `LlmMessage::System` 变体）
- `crates/runtime-llm/`（compact 专用的 LLM 调用）
- `crates/runtime-config/`（新增 `compact_threshold_percent`、`tool_result_max_bytes`、token budget 相关配置项）

---

## P4：错误恢复

### 目标

对可恢复的 API 错误（413 Prompt Too Long、Max Output Tokens）进行自动重试，而非直接终止 turn。

### 需求

#### P4.1 413 Prompt Too Long → Reactive Compact

当 LLM provider 返回 prompt-too-long 错误时：

1. 不终止 turn，而是执行一次紧急 compact
2. compact 成功后重新进入 step 循环（不消耗 step_index）
3. 如果 compact 后仍然 413，放弃并返回错误
4. 单次 turn 最多尝试 1 次 reactive compact（防止循环）

```rust
// 在 generate_response 的错误处理中
Err(e) if is_prompt_too_long(&e) => {
    if !has_attempted_reactive_compact {
        messages = reactive_compact(messages, &provider, &system_prompt).await?;
        has_attempted_reactive_compact = true;
        continue;  // 重试当前 step
    }
    return finish_with_error(turn_id, "prompt too long after compact", on_event);
}
```

#### P4.2 Max Output Tokens → 重试

当 LLM 因 max_output_tokens 截断时：

1. 检测截断信号（provider 返回 `finish_reason: "max_tokens"` 或类似标记）
2. 向 messages 追加一条 system nudge：`[The previous response was truncated. Continue from where you left off.]`
3. 重新调用 LLM，不消耗 step_index
4. 单次 turn 最多重试 3 次

#### P4.3 错误类型分类

在 `crates/runtime-llm` 中引入结构化错误类型：

```rust
pub enum LlmError {
    /// 请求内容超出 context window
    PromptTooLong { token_count: Option<usize> },
    /// 响应被 max_output_tokens 截断
    OutputTruncated { finish_reason: String },
    /// 速率限制
    RateLimited { retry_after: Option<Duration> },
    /// 其他不可恢复错误
    Other(anyhow::Error),
}
```

### 验收标准

- [ ] 模拟 413 错误时自动触发 reactive compact 并恢复
- [ ] 模拟 max_output_tokens 截断时自动继续生成
- [ ] 恢复失败时正确终止 turn 并返回 `TurnOutcome::Error`
- [ ] 重试次数有上限，不会无限循环

### 影响范围

- `crates/runtime/src/agent_loop/turn_runner.rs`（错误恢复逻辑）
- `crates/runtime-llm/`（结构化错误类型）
- 依赖 P3 的 compaction 模块

---

## P5：API 韧性层

### 目标

降低 API 调用成本（prompt caching）和提升可用性（模型降级、速率限制处理）。

### 需求

#### P5.1 Cache Breakpoints

在 `runtime-llm` 的消息组装层，为支持 prompt caching 的 provider（Anthropic、OpenAI 等）自动插入 cache marker：

**Single-marker 策略**：每次请求只在**最后一条消息**上放一个 `cache_control` marker，而非到处放。原因：服务端 cache 以 prefix 为单位释放，中间位置放 marker 浪费 KV 页。

```rust
fn apply_cache_breakpoints(messages: &mut [LlmMessage]) {
    // 清除所有已有 marker
    for msg in messages.iter_mut() {
        msg.cache_control = None;
    }
    // 只在最后一条消息上标记
    if let Some(last) = messages.last_mut() {
        last.cache_control = Some(CacheControl::Ephemeral);
    }
}
```

**Config 配置**：

```toml
[prompt]
# prompt cache TTL（仅部分 provider 支持）
cache_ttl_minutes = 5
```

#### P5.2 Model Fallback

当主模型返回 529 (overloaded) 连续 3 次时，自动切换到备选模型：

```rust
async fn attempt_with_fallback(
    provider: &dyn LlmProvider,
    request: ModelRequest,
    fallback_model: Option<&str>,
    max_529_retries: usize,
) -> Result<LlmOutput> {
    let mut retry_count = 0;
    loop {
        match provider.generate(request.clone()).await {
            Ok(output) => return Ok(output),
            Err(LlmError::Overloaded) => {
                retry_count += 1;
                if retry_count >= max_529_retries {
                    if let Some(fallback) = fallback_model {
                        // 切换模型，剥离 thinking signatures
                        let cleaned = strip_thinking_signatures(request.messages);
                        return fallback_provider.generate(request.with_model(fallback)).await;
                    }
                    return Err(LlmError::Overloaded);
                }
                tokio::time::sleep(backoff(retry_count)).await;
            }
            Err(e) => return Err(e),
        }
    }
}
```

关键细节：
- 切换模型时**剥离 thinking signatures**（模型绑定的加密数据，换模型会 400）
- 切换通过 `StorageEvent` 或 system message 通知用户
- 可配置 `fallback_model`（如 `"claude-sonnet"`）

#### P5.3 Rate Limit 处理

对 429 响应的分级处理：

1. `retry_after < 30s`：sleep 后重试（保持同一模型）
2. `retry_after >= 30s` 或无 `retry_after`：切换到备选模型（如有）
3. 无备选模型：sleep 指数退避（最大 5 分钟），并在等待期间 yield heartbeat 防止连接超时

```rust
fn handle_rate_limit(retry_after: Option<Duration>, has_fallback: bool) -> RateLimitAction {
    match (retry_after, has_fallback) {
        (Some(d), _) if d < Duration::from_secs(30) => RateLimitAction::RetryAfter(d),
        (_, true) => RateLimitAction::Fallback,
        _ => RateLimitAction::Backoff(Duration::from_secs(5 * 60)),
    }
}
```

### 验收标准

- [ ] 每次 API 请求自动在最后一条消息插入 cache marker
- [ ] 主模型 529 超过 3 次后自动切换备选模型
- [ ] 切换模型后 thinking signatures 被正确剥离
- [ ] 429 响应按 retry_after 时长分级处理
- [ ] 用户收到模型切换的通知

### 影响范围

- `crates/runtime-llm/`（cache marker 插入、fallback 逻辑、rate limit 处理）
- `crates/runtime/src/agent_loop/turn_runner.rs`（集成 fallback 返回的输出）
- `crates/runtime-config/`（新增 `fallback_model`、`cache_ttl_minutes` 配置项）

---

## P6：子 Agent（远期占位）

### 核心设计：递归 AgentLoop

子 agent 不是新架构，而是 **AgentTool 创建并运行一个子 AgentLoop**。

```rust
impl Tool for AgentTool {
    async fn execute(&self, ctx: ToolContext, args: Value) -> Result<ToolExecutionResult> {
        let task = args["task"].as_str().unwrap_or_default();
        let child_cancel = ctx.cancel.child();  // 父取消 → 子取消
        let child_loop = AgentLoop::from_capabilities(
            self.factory.clone(),
            self.capabilities.clone(),
        ).with_agent_depth(self.depth + 1);

        let child_state = AgentState {
            messages: vec![LlmMessage::User { content: task.to_string() }],
            working_dir: ctx.working_dir.clone(),
            ..Default::default()
        };
        let outcome = child_loop.run_turn(&child_state, "sub-turn", &mut noop_sink, child_cancel).await?;
        Ok(ToolExecutionResult { ok: true, output: outcome.final_text(), .. })
    }
}
```

**为什么这样做是对的**：
- 生命周期天然正确：CancelToken 是 Clone 的，父没了子自动停
- 和 StepHook 天然兼容：子 agent 注册自己的 hooks，父 agent 的 hooks 不污染子 agent
- 不需要 Swarm 框架，不需要 TeamCoordinator

### 三种协作模式

递归 AgentLoop + P2 并行执行天然覆盖三种模式，无需额外架构：

| 模式 | 实现 | 说明 |
|------|------|------|
| **委派** | AgentTool（单次调用） | 父 agent 等子 agent 完成，结果作为 ToolResult 返回 |
| **并行团队** | 多个 AgentTool 并发调用 | P2 的 `concurrency_safe` 让多个 AgentTool 并行执行，各自独立上下文，结果汇总给父 agent(leader) |
| **接替** | 跨 turn 上下文传递 | 父 turn 结束，子 turn 继续（通过 session event 流衔接） |

### 关键设计决策

#### 共享与隔离

| 资源 | 策略 | 原因 |
|------|------|------|
| `CapabilityRouter` | 共享 Arc | 子 agent 可调用相同工具 |
| `readFileState` 缓存 | 共享 Arc\<RwLock\> | 避免重复读文件，写入时 invalidation |
| `messages` | 完全隔离 | 子 agent 不污染父对话 |
| `CancelToken` | child token | 父取消传播到子，子取消不影响父 |
| `StepHook` | 独立注册 | 子 agent 可用更宽松的 hooks |
| `PolicyEngine` | 包装降级 | 子 agent 自动批准读操作，保留高风险审批 |

#### Prompt Cache 复用

子 agent 的 system prompt 和 tool definitions 与父一致 → P5 的 cache breakpoints 让 LLM provider 的 prompt cache 直接命中，避免重复付费。

#### 递归深度限制

```rust
const MAX_AGENT_DEPTH: u8 = 3;

// AgentLoop 持有 depth counter
pub struct AgentLoop {
    // ...existing
    agent_depth: u8,  // 0 = 顶层, 1 = 子 agent, ...
}
// 超过 MAX_AGENT_DEPTH 时 AgentTool 返回错误
```

#### Approval 降级

子 agent 继承父 agent 的授权上下文，读操作自动批准，仅高风险操作（shell、文件写入）保留审批：

```rust
let child_policy = AutoApproveReadPolicy::wrap(parent_policy.clone());
```

### 已知弊端与缓解

| 弊端 | 缓解 |
|------|------|
| 每个 sub-agent 重复 system prompt token | P5 prompt cache 命中，实际只多付一次 |
| 子 agent 编辑文件后父 agent 缓存过期 | 共享 `readFileState` + 写入时 invalidation |
| 递归过深 | `MAX_AGENT_DEPTH = 3` 硬限制 |
| 子 agent 审批弹窗干扰用户 | `AutoApproveReadPolicy` 降级 |

### 参考

- Claude Code：`createSubagentContext()` + `runForkedAgent()`
- Claude Code Swarm：多 agent 通过 `setAppState` 共享状态
- Astrcode 的实现更简洁：**AgentTool + P2 并行 = 天然团队**，无需独立 Swarm 框架

> 建议在 P1-P3 稳定后再启动设计。

---

## 依赖关系

```
P1 (状态机化)
 ├── P2 (并行工具执行)
 │    └── 无进一步依赖
 └── P3 (上下文压缩 + Token Budget)
      ├── P4 (错误恢复，依赖 P3 的 compaction)
      └── P5 (API 韧性，可与 P4 并行)

P6 (子 Agent，独立于 P2-P5，远期占位)
```

- P1 是所有后续阶段的基础
- P2 和 P3 可并行开发
- P4 依赖 P3（reactive compact 复用 compact 模块）
- P5 可与 P4 并行（API 韧性不依赖 compaction）
- P6 远期占位，等 P1-P3 稳定后再启动

---

## 远期 TODO

以下模式经过评估，有价值但优先级较低，待核心架构（P1-P5）稳定后再展开设计。

### Stop Hooks（后处理扩展点）

> **TODO**: 每次 turn 结束后运行的后处理钩子框架。
>
> Claude Code 在 turn 完成后执行多个异步任务：
> - 用户自定义 shell hooks（可阻止继续）
> - Prompt Suggestion（用小模型预测用户下一步，生成 2-12 字建议）
> - Memory Extraction（从对话中提取记忆）
> - Cache State Snapshot（保存 prompt cache 上下文）
>
> 预留扩展点：
> ```rust
> // turn_runner.rs turn 结束时
> async fn run_stop_hooks(
>     turn_outcome: &TurnOutcome,
>     state: &AgentState,
>     hooks: &[Arc<dyn StopHook>],
> ) -> Result<StopHookDecision>
> ```
>
> 参考：Claude Code 的 `src/query/stopHooks.ts`
> 优先级：**中**。用户自定义 hooks 框架优先，suggestion 和 memory 后面加。

### Streaming Tool Execution（流式工具执行）

> **TODO**: 不等 LLM 响应完全接收就开始执行工具。
>
> 当 LLM 流式输出 5 个 tool_call 时，第 1 个的 JSON 传完就可以开始执行，
> 同时继续接收剩余的。多工具场景可省几秒。
>
> 关键设计点：
> - 依赖 P2 的并行执行基础设施
> - 需要在 streaming response 中识别完整的 tool_use block
> - `sibling abort cascade`：并行 Bash 命令中一个失败时取消所有兄弟进程
> - `discard()` 方法：fallback 时放弃已启动的部分工具
>
> 参考：Claude Code 的 `StreamingToolExecutor.ts`
> 优先级：**低**。需 P2 完成后才能启动，实现复杂度较高。

### Tool Use Summaries（工具摘要）

> **TODO**: 用小模型为每个工具调用生成 30 字摘要。
>
> 主要用于非交互/SDK 消费者（移动端 UI、IDE 插件）显示 agent 进度。
> 核心对话循环不依赖此功能。
>
> 参考：Claude Code 的 `toolUseSummaryGenerator.ts`
> 优先级：**低**。纯 UX 优化。

---

## 附录：Claude Code 参考架构

| 模式 | Claude Code 实现 | Astrcode 对应阶段 |
|------|-----------------|------------------|
| State + Terminal | `State` 类型 + `Terminal` 返回 | P1 |
| 并行工具执行 | `StreamingToolExecutor` + batch partition | P2 |
| Tool Result Budget | `applyToolResultBudget()` | P3.3 |
| Auto-Compact | 5 层管线（budget → snip → micro → collapse → auto） | P3.4 |
| Token Budget / Auto-Continue | 预算解析 + nudge 消息 + diminishing-returns 检测 | P3.6 |
| Reactive Compact | 413 后紧急 compact + 重试 | P4.1 |
| Max Output Tokens 恢复 | 重试 + max_tokens 升级到 64k | P4.2 |
| Cache Breakpoints | single-marker + TTL + header latching | P5.1 |
| Model Fallback | 529 计数 + 模型切换 + thinking signature 剥离 | P5.2 |
| Rate Limit 处理 | 分级 retry_after + fallback + backoff | P5.3 |
| Stop Hooks | 用户 hooks + suggestion + memory extraction | 远期 TODO |
| Streaming Tool Execution | 流式接收中提前执行工具 | 远期 TODO |
| Tool Use Summaries | 小模型生成工具摘要 | 远期 TODO |
| Sub-Agent | `createSubagentContext()` + `runForkedAgent()` | P6：递归 AgentLoop |
| Agent Team / Swarm | 多 agent 共享 `setAppState` | P6：AgentTool + P2 并行，无需独立 Swarm 框架 |
