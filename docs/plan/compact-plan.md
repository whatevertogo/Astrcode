# Compact 系统优化计划

> 基于对五个主流 coding agent（Claude Code, Codex, OpenCode, Kimi CLI, pi-mono）的深度对比分析
> 结合 Astrcode 现有架构代码审查结果，进行优先级排序

---

## 一、对比分析总览

### 1.1 各方案核心优势

| 项目 | 核心优势 | 值得借鉴的点 |
|------|---------|-------------|
| **Claude Code** | 工业级稳定性 | 5 层架构（Auto/Micro/Time/API/Session Memory）、电路熔断、Post-compact 附件恢复（文件+Plan+Skill+MCP）、Cache-sharing fork、时间微压缩（cache TTL 感知） |
| **Codex** | 灵活的压缩方向 | Mid-turn 压缩、InitialContextInjection（前缀/后缀注入）、OpenAI remote compact API、Ghost Snapshot 保持 /undo 可用 |
| **OpenCode** | 三层渐进压缩 | pruned→process→create 三层工作流、专用 compact agent（无工具权限）、Auto-continue 合成消息、Prune 标记机制（可审计） |
| **pi-mono** | 增量摘要 | Update prompt 模式（旧摘要+新内容→合并）、Branch 摘要导航（/tree）、文件操作跟踪、Extension hook（session_before_compact） |
| **Kimi CLI** | 简洁有效 | 50K reserved buffer 简单可靠、压缩内容优先级排序（当前任务>错误>代码>上下文>设计决策>TODO）、JSONL 检查点 |
| **Astrcode** | 架构清晰 | Policy/Strategy/Rebuilder 三层分离、ContextPipeline stage 管线、413 reactive compact、已实现电路熔断 |

> **注意**：通过代码审查确认，Astrcode **已经实现**了以下曾被列为"缺失"的功能：
> - ✅ **电路熔断**：`ThresholdCompactionPolicy.consecutive_failures`, 3 次连续失败熔断
> - ✅ **Post-compact 附件恢复**：`FileAccessTracker` + `recover_file_contents()`（5 个文件，50K token 预算）
> - ✅ **Auto-continue Nudge**：`AutoContinueNudge` 消息
> - ✅ **锚点 Token 计数**：`TokenUsageTracker.anchored_budget_tokens`
> - ✅ **增量重压缩**：`CompactSummary` 消息保留在前缀中，`compact_input_messages()` 保留前一个摘要
> - ✅ **413 降级重试**：`drop_oldest_turn_group()` 最多 3 次重试
> - ✅ **微压缩**：`microcompact.rs`（截断 + 清除可清除工具）
> - ✅ **Live tail 录制**：`CompactionTailSnapshot` 处理 reactive compact

### 1.2 真正缺失的能力

| 缺失项 | 影响 | 参考项目 | 优先级 |
|--------|------|---------|--------|
| Compact Hook 系统 | 🔴 高 — 插件无法介入压缩流程，无扩展性 | Claude Code, pi-mono | 🔴 Phase 1 |
| Prune 标记机制 | 🟡 中 — 前缀事件直接丢弃，不可审计 | OpenCode | 🟢 Phase 2 |
| 时间触发微压缩 | 🟢 低 — cache 过期时不清理旧工具结果 | Claude Code | 🟢 Phase 2 |
| Partial Compact 方向 | 🟢 低 — 只支持后缀保留，不支持 from 前缀保留 | Claude Code, Codex | 🟢 Phase 2 |
| Prompt 工程升级 | 🟢 低 — 缺少标签清理、analysis 校验、优先级排序 | Claude Code, Kimi CLI | 🟢 Phase 2 |
| 精确 Token 计数 | 🟡 中 — 4 chars/token 启发式 + 锚点，无真正 tokenizer | 全部项目 | 🟡 Phase 2 |
| 文件操作跟踪 | 🟢 低 — 摘要中无结构化文件读写信息 | pi-mono | 🔵 Phase 3 |
| Ghost Snapshot | 🔵 低 — 压缩后 /undo 可能受影响 | Codex | 🔵 Phase 3 |
| Cache-sharing fork | 🟡 中 — 压缩无法复用主对话 cache | Claude Code | 🟢 Phase 2 |
| Context Usage 可视化 | 🔵 低 — 前端无 token 分布指示 | Claude Code, OpenCode | 🔵 Phase 3 |
| ~~多模态消息处理~~ | 🟢 暂缓 — 当前不支持多模态，预留钩子 | Claude Code, Codex | ⏸️ Phase 3+ |
| ~~专用 Compact Agent~~ | 🟢 暂缓 — 需 agent-tool 重构后实现 | OpenCode | ⏸️ Phase 4 |
| Mid-turn 压缩 | 🔵 低 — 无法在 model response 中间超限处理 | Codex | 🔵 Phase 3+

---

## 二、优化阶段规划

### Phase 1：Hook 系统（扩展性加固）

> **目标**：为压缩流程增加插件扩展点，后续所有优化均可通过 Hook 介入

#### 1.1 Compact Hook 系统

**问题**：插件无法介入压缩流程。无法自定义压缩 prompt、无法在压缩前修改上下文、无法提供自定义摘要、无法在压缩后执行恢复操作。

**Claude Code 参考**：PreCompact / PostCompact / SessionStart 三类 hook。

**pi-mono 参考**：`session_before_compact` event（event hook 可取消或提供自定义摘要）。

**设计方案**：
```rust
// core 中定义 Hook 事件类型
pub struct CompactHookEvent<'a> {
    pub messages: &'a Vec<LlmMessage>,
    pub system_prompt: &'a str,
    pub tools: &'a [ToolDefinition],
    pub trigger: CompactionReason,
}

pub struct CompactHookResult {
    pub override_system_prompt: Option<String>,
    pub override_messages: Option<Vec<LlmMessage>>,
    pub override_tools: Option<Vec<ToolDefinition>>,
    pub cancel: bool,  // PreCompact 可取消
}

// Hook 类型
pub enum CompactPhase {
    PreCompact,    // 压缩前，可修改 prompt/messages/tools，可取消
    PostCompact,   // 压缩后，可执行恢复操作
}

// hooks 接口 trait
#[async_trait]
pub trait CompactHook: Send + Sync {
    fn phase(&self) -> CompactPhase;
    async fn on_event(&self, event: &CompactHookEvent<'_>) -> Result<CompactHookResult>;
}

// auto_compact() 前后调用：
//   → PreCompact hooks 链式执行，收集修改
//   → 应用修改后的 prompt/messages/tools
//   → 执行 LLM 压缩
//   → PostCompact hooks 链式执行，执行恢复
```

**实现要点**：
- 复用 `crates/plugin/` 的 JSON-RPC 通信通道
- 参考现有 `PolicyHook` 的注册和链式执行机制
- PreCompact hook 支持取消（`cancel: true`），跳过本次压缩
- PostCompact hook 可请求读取文件恢复附件

**涉及文件**：
- `crates/core/src/`（compact hook 事件类型）
- `crates/runtime-agent-loop/src/compaction_runtime.rs`
- `crates/plugin/`（hook 注册和通信）

---

### Phase 3：智能化 & 高级场景覆盖（含暂缓项）

> **目标**：提升摘要质量、扩展性、可审计性，以及未来多模态扩展的预留

#### 3.1 文件操作跟踪

### Phase 2：Cache 优化 & 压缩方向 & Prompt 工程

> **目标**：利用 Prompt Cache 减少成本和延迟；支持灵活压缩方向；提升摘要质量

**问题**：压缩时发送独立请求，无法复用主对话的 cached prefix，浪费 token。Claude Code 的 fork agent 复用主对话 cached prefix，节约 30-50% token，98% 缓存命中率。

**设计方案**：
```rust
// LlmProvider trait 扩展
pub trait LlmProvider: Send + Sync {
    // 新增方法（默认 impl 返回 false）
    fn supports_cache_sharing(&self) -> bool { false }
    fn fork_for_compaction(&self) -> Option<Self> { None }
}

// Anthropic Provider 实现
impl LlmProvider for AnthropicProvider {
    fn supports_cache_sharing(&self) -> bool {
        true  // Anthropic 有 cache_control: ephemeral
    }

    fn fork_for_compaction(&self) -> Option<Self> {
        // 复用 HTTP 连接，压缩请求发送相同的 system prompt（带 cache_control）
        // + 相同的前缀消息（带 cache_control），命中 KV cache
        Some(self.clone())
    }
}

// OpenAI Provider：
//   supports_cache_sharing() = false，走独立请求路径
//   未来可利用 OpenAI automatic cache（相同前缀自动命中）
```

**auto_compact() 中走 cache-sharing 路径**：
```rust
if provider.supports_cache_sharing() {
    let forked = provider.fork_for_compaction();
    // 使用 forked provider，消息与主对话共享相同的前缀 + system prompt
} else {
    // 回退到当前独立请求路径
}
```

**涉及文件**：
- `crates/runtime-llm/src/lib.rs`（LlmProvider trait）
- `crates/runtime-agent-loop/src/context_window/compaction.rs`
- `crates/runtime-llm/src/anthropic.rs`（Anthropic Provider 实现 cache_control）

---

#### 2.3 时间触发微压缩

**问题**：当前微压缩仅按 token 压力触发。当 server 端 prompt cache 已过期时，清除旧工具结果不会浪费缓存。

**Claude Code 参考**：超过 30 分钟（默认 60 分钟，1h TTL）清除旧工具结果。

**设计方案**：
```rust
pub enum MicrocompactTrigger {
    TokenPressure,  // 现有：token 压力
    CacheExpired,   // 新增：cache 过期
    Both,           // 两者同时
}

// 检查逻辑：
fn should_cache_expire_compact(
    last_assistant_ts: DateTime<Utc>,
    now: DateTime<Utc>,
    threshold_seconds: u64,  // 默认 900 (15分钟)
) -> bool {
    (now - last_assistant_ts).num_seconds() > threshold_seconds as i64
}

// 清除策略与现有 truncate/clear 一致
```

**配置暴露**：
```toml
[microcompact]
time_threshold_seconds = 900  # 15 分钟
keep_recent_turns = 3         # 保留最近 3 个 turn 不清除
```

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/microcompact.rs`
- `crates/runtime-config/src/`（配置）

---

#### 2.3 Prune 机制（OpenCode 风格）

**问题**：微压缩仅在 LlmMessage 级别操作，不会在持久化存储层清理已完成 turn 的旧工具结果。OpenCode 的 prune 从后往前扫描，标记而非删除原始内容。

**OpenCode 参考**：prune() 保护最近 40K tokens 工具输出，超出替换占位；标记不删除（可审计）。

**设计方案**：
```rust
// context pipeline 新增 PostTurnPruneStage
pub struct PostTurnPruneConfig {
    pub prune_protect_tokens: usize,   // 保护阈值（默认 40K）
    pub prune_minimum: usize,          // 最小修剪量（默认 20K，低于不执行）
    pub protected_tools: HashSet<String>,  // 特定工具不 prune（如 skill）
}

fn prune_old_tool_results(
    events: &[StoredEvent],
    config: &PostTurnPruneConfig,
) -> Vec<StoredEvent> {
    // 从后往前累加 token
    // 超出保护区的旧 tool result 替换为占位文本
    // 保留原始事件但修改 content 为 "[Tool result pruned]"
}
```

**涉及文件**：
- `crates/runtime-agent-loop/src/context_pipeline.rs`
- `crates/runtime-agent-loop/src/context_window/`（新增 prune.rs）
- `crates/storage/src/`（StoredEvent 修改）

---

#### 2.4 Partial Compact 方向

**问题**：只支持后缀保留压缩（`up_to`）。Claude Code 支持 partialCompact 两种方向：
- `from`：保留前缀，压缩后缀 —— **cache 友好**（前缀消息的 KV cache 仍然有效）
- `up_to`：保留后缀，压缩前缀 —— 当前唯一支持的模式

**Claude Code / Codex 参考**：`from` 方向用于 cache 友好压缩，`up_to` 用于需要保留最新上下文的场景。

**设计方案**：
```rust
pub enum CompactDirection {
    /// 保留前缀（从 index 开始压缩后缀），cache 友好
    From { from_index: usize },
    /// 保留后缀（压缩 index 之前的前缀），当前默认行为
    UpTo { up_to_index: usize },
}

// CompactConfig 扩展
pub struct CompactConfig {
    pub direction: CompactDirection,  // 新增
    pub keep_recent_turns: usize,
    pub trigger: CompactTrigger,
}

// split_for_compaction() 按方向分割：
fn split_for_compaction(
    messages: &[LlmMessage],
    direction: &CompactDirection,
) -> (Vec<LlmMessage>, Vec<LlmMessage>)  // (compacted_prefix, preserved_suffix)
```

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/compaction.rs`
- `crates/runtime-agent-loop/src/compaction_runtime.rs`
- `crates/core/src/action.rs`

---

#### 2.6 Prompt 工程升级

**问题**：当前 `build_compact_system_prompt` 已有 9 段结构，但缺少：
- `<analysis>` scratchpad 的存在性校验（Claude Code）
- 标签清理（去除残留 XML 标签）
- 压缩内容优先级排序（Kimi CLI）
- NO_TOOLS 约束的强化措辞

**Claude Code 参考**：
```
Analysis scratchpad + NO_TOOLS 约束（移到最后 user message 前面）
formatCompactSummary(): 清理 XML 标签残留，规范化空白
```

**Kimi CLI 参考**：
```
压缩优先级排序（当前任务 > 错误修复 > 代码演进 > 系统上下文 > 设计决策 > TODO）
```

**设计方案**：
```rust
// 1. extract_summary() 增加 analysis 块校验
fn extract_summary(response: &str) -> Result<String> {
    let analysis = extract_analysis(response)
        .inspect(|_| log::trace!("compression: analysis block present"))
        .unwrap_or_default();
    if analysis.is_empty() {
        log::warn!("compression: missing <analysis> block — summary quality may be low");
    }
    extract_summary_block(response)?
}

// 2. format_compact_summary() 新增标签清理
pub fn format_compact_summary(summary: &str) -> String {
    // 清理残留 XML 标签
    let cleaned = summary
        .replace("<summary>", "")
        .replace("</summary>", "")
        .replace("<analysis>", "")
        .replace("</analysis>", "")
        .trim();
    // 规范化空白：多个连续空白合并为一个
    let normalized = regex_replace(r"\s+", " ", cleaned);
    format!(
        "[Auto-compact summary]\n{}\n\nContinue from this summary without repeating it to the user.",
        normalized
    )
}

// 3. build_compact_system_prompt() 强化
//    - NO_TOOLS 约束用更强硬措辞
//    - 内容优先级排序明确写出
//    - 支持通过 Hook 注入自定义指令（Phase 1.2）
```

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/compaction.rs`（build/extract/format）
- `crates/core/src/projection/agent_state.rs`（format_compact_summary）

---

#### 2.7 精确 Token 计数

**问题**：当前 4 chars/token 启发式误差可达 ±30%。虽然 `TokenUsageTracker` 已有 `anchored_budget_tokens` 锚点机制，但 context 估算仍是启发式。

**各方案参考**：
- Claude Code: `roughTokenCountEstimation()` + Provider usage 锚定
- pi-mono: 优先使用最后 assistant 消息的 usage，回退估算
- Codex: `approx_token_count()` = `text.len() / 4`

**设计方案**：
```rust
// TokenUsageTracker 扩展（短期）
pub struct TokenUsageTracker {
    anchored_budget_tokens: usize,           // 现有：Provider total_tokens 累积
    last_input_tokens: Option<usize>,        // 新增：Provider input_tokens
    anchor_message_index: Option<usize>,     // 新增：锚点消息索引
    anchor_timestamp: Option<Instant>,       // 新增：锚点时间
}

impl TokenUsageTracker {
    pub(crate) fn record_usage(&mut self, usage: Option<LlmUsage>) {
        if let Some(u) = usage {
            self.anchored_budget_tokens = self.anchored_budget_tokens
                .saturating_add(u.total_tokens());
            self.last_input_tokens = Some(u.input_tokens);  // 新增
            self.anchor_message_index = Some(self.last_message_index());  // 新增
        }
    }

    /// 锚点增强估算（中期）
    pub fn estimate_context_tokens(&self) -> usize {
        if let (Some(input_tokens), Some(anchor_idx)) =
            (self.last_input_tokens, self.anchor_message_index)
        {
            // 锚点到当前消息的增量用 4 chars/token 估算
            let incremental = estimate_text_since_anchor(anchor_idx);
            input_tokens.saturating_add(incremental)
        } else {
            // 无锚点：回退全量估算
            self.budget_tokens(estimate_total_tokens())
        }
    }
}
```

**中期（中期交付）**：
- 为 OpenAI-compatible Provider 集成 `tiktoken` tokenizer
- 为 Anthropic Provider 复用 Anthropic 自身的 token 返回

**长期**：
- `LlmProvider` trait 增加 `tokenize(&str) -> Vec<u32>` 方法
- context 估算完全切换为 Provider-native

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/token_usage.rs`
- `crates/runtime-llm/src/lib.rs`

---

### Phase 3：智能化（高级场景覆盖）

> **目标**：提升摘要质量、扩展性、可审计性

#### 3.4 Cache-sharing Fork 压缩

**问题**：压缩时发送独立请求，无法复用主对话的 cached prefix，浪费 token。Claude Code 的 fork agent 复用主对话 cached prefix，节约 30-50% token，98% 缓存命中率。

> **说明**：本项与 Phase 2 的 Cache-sharing fork 为同一功能，因实现依赖 Provider trait 扩展，与 Phase 2 中其他项合并执行，此处保留完整设计参考。

（设计内容同下方 Phase 2.1）

#### 3.5 多模态消息处理（暂缓）

> **状态**：⏸️ 当前项目不支持多模态，仅预留设计。待 `ContentPart` 扩展后再实现。

**问题**：当未来 `LlmMessage` 扩展为 `Vec<ContentPart>`（包含 Image/Document）时，压缩时将多模态内容直接发给 LLM 生成摘要可能因 prompt 过长而失败。

**Claude Code 参考**：`stripImagesFromMessages()` 将 Image/Document 替换为占位符。

**设计方案**：
```rust
// content_part.rs（未来扩展）
fn strip_multimodal_for_compact(content: &ContentPart) -> String {
    match content {
        ContentPart::Text(t) => t.clone(),
        ContentPart::Image(_) => "[image]".to_string(),
        ContentPart::Document(_) => "[document]".to_string(),
    }
}

// 在 compact_input_messages() 中应用
```

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/compaction.rs`
- `crates/core/src/action.rs`（LlmMessage 多模态扩展时适配）

#### 3.6 专用 Compact Agent（暂缓）

> **状态**：⏸️ 需 agent-tool 重构后实现。当前 `auto_compact()` 直接调用主模型，system prompt 已有 NO_TOOLS 约束，短期可接受。

**问题**：直接调用主模型生成摘要，LLM 可能忍不住调用工具（尽管有 NO_TOOLS 约束），浪费 token。

**OpenCode 参考**：专用 compact agent，无工具权限，可配置更低成本的模型。

**设计方案**：等 agent-tool 重构完成后，复用其 Agent 抽象创建无工具权限的 compact-only 实例。

#### 3.7 Prune 标记机制（OpenCode 风格）

**问题**：当前 `auto_compact()` 直接调用主模型生成摘要，LLM 可能忍不住调用工具（尽管 system prompt 有 NO_TOOLS 约束），浪费 token 且增加不确定性。

**OpenCode 参考**：专用 compact agent，无工具权限，使用特定模型（可配置更低成本的模型）。

**设计方案**：
```rust
// 配置扩展
pub struct CompactionConfig {
    pub dedicated_agent_model: Option<String>,  // 例如 "claude-haiku"
    pub dedicated_agent_tools: bool,            // 默认 false（无工具）
}

// auto_compact() 中使用专用 agent
let agent_for_compact = if config.dedicated_agent_model.is_some() {
    provider.spawn_for_model(&config.dedacted_agent_model)?
} else {
    current_provider.clone()
};
// 不传递工具定义（tools = &[]）
```

**涉及文件**：
- `crates/runtime-agent-loop/src/compaction_runtime.rs`
- `crates/runtime-config/src/`（配置）

---

#### 3.8 Ghost Snapshot（Undo 保护）

**问题**：当前压缩后前缀事件被移除，`/undo` 可能无法恢复到 compact 前的完整状态。Codex 通过 Ghost Snapshot 保持 undo 历史。

**Codex 参考**：
```rust
// 原始历史 → Compact → 摘要 + GhostSnapshots
let ghost_snapshots: Vec<ResponseItem> = history
    .iter()
    .filter(|item| matches!(item, ResponseItem::GhostSnapshot { .. }))
    .cloned()
    .collect();
// GhostSnapshot::Event 包含被压缩掉的事件原文
```

**设计方案**：
```rust
// StorageEvent 扩展变体
pub enum StorageEvent {
    // ... 现有变体
    /// Ghost snapshot，仅用于 undo 恢复，不参与对话上下文
    GhostSnapshot {
        original_events: Vec<StoredEvent>,
        compact_seq_range: (u64, u64),  // 被压缩的事件序号范围
    },
}

// auto_compact() 后保存 snapshot
let snapshot = StorageEvent::GhostSnapshot {
    original_events: compacted_events.clone(),
    compact_seq_range: (source_range.start, source_range.end),
};
session_repository.append(&snapshot)?;
```

**实现要点**：
- GhostSnapshot 不参与对话上下文投影（`project()` 中 skip）
- 仅在 `/undo` 或手动回滚时提取
- 存储开销：一次 compact 约额外存储 5-10K tokens（可配置保留最近的 N 次 snapshot）

**涉及文件**：
- `crates/core/src/event/types.rs`（StorageEvent 扩展）
- `crates/storage/src/session.rs`
- `crates/runtime-agent-loop/src/context_window/compaction.rs`

---

#### 3.3 文件操作跟踪

**问题**：摘要中缺乏对读写文件的结构化跟踪。LLM 生成摘要时不会显式列出 read/write/edited 过的文件。

**pi-mono 参考**：`extractFileOperations()` 累积跟踪 read/written/edited 文件，摘要追加 `<read-files>` 和 `<modified-files>` XML 段。

**设计方案**：
```rust
#[derive(Debug, Default)]
pub struct FileOperationSet {
    pub read: Vec<PathBuf>,
    pub written: Vec<PathBuf>,
    pub edited: Vec<PathBuf>,
}

impl FileOperationSet {
    /// 从 LlmMessage 中提取工具调用中的文件路径
    fn extract_from_messages(messages: &[LlmMessage]) -> Self {
        for msg in messages {
            if let LlmMessage::Assistant { tool_calls, .. } = msg {
                for call in tool_calls {
                    // 从 args JSON 中提取 path/paths 参数
                }
            }
        }
    }

    /// 跨多次压缩累积（增量合并，去重）
    fn merge(&mut self, other: &Self) {
        // ...
    }
}

// 在摘要 prompt 中注入：
//   "Files read: path1, path2\nFiles modified: path3, path4"
// 在摘要输出中追加 XML 段
```

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/compaction.rs`
- `crates/runtime-agent-loop/src/compaction_runtime.rs`

---

#### 3.9 Mid-turn 压缩

**问题**：当前仅在 turn 执行结束后检测超限。在 model response 中间（特别是大工具结果或长文本）遇到 context 超限时无法及时处理。

**Codex 参考**：Mid-turn 压缩在 model response 完成后检测 token 超限，inline 触发压缩。

**设计方案**：
```rust
// AgentLoop step 中检测
async fn step(&mut self) -> Result<StepResult> {
    let output = self.provider.generate(request, sink).await?;

    // 检测当前 context 是否超限
    if self.token_tracker.estimate_context_tokens() >= threshold {
        // Mid-turn 压缩：在当前 turn 内就地压缩
        self.compact_runtime
            .compact_mid_turn(self.tail_snapshot.clone())?;
        // 压缩完成后恢复该 turn 的后续执行
    }
}
```

**实现要点**：
- 需要 `CompactionTailSnapshot` 支持 mid-turn 场景（录制当前 turn 已产生的事件）
- 压缩后将 compacted 消息替换到上下文，继续当前 turn
- 需要处理工具调用的连续性（一个工具调用跨 compact 边界）

**涉及文件**：
- `crates/runtime-agent-loop/src/agent_loop.rs`
- `crates/runtime-agent-loop/src/compaction_runtime.rs`

---

#### 3.10 Context Usage 可视化

**问题**：前端无 token 分布指示，用户无法看到上下文使用情况。

**Claude Code 参考**：ContextVisualization 组件展示 token 按类别分布、`/context` 命令。

**设计方案**：
```
Tauri command: get_context_usage() → ContextUsage {
    total_tokens: usize,
    context_window: usize,
    usage_percent: f64,
    breakdown: {
        system_prompt: usize,
        tools: usize,
        history: usize,
        files: usize,
    }
}

前端 context usage 指示器（进度条 + tooltip）
压缩前显示 token 节省预估（pre_tokens vs post_tokens_estimate）
```

**涉及文件**：
- `src-tauri/src/commands.rs`
- `frontend/src/components/`（ContextUsage 组件）
- `crates/server/src/`（API 端点）

---

## 三、实施优先级总览

| 阶段 | 事项 | 优先级 | 难度 | 收益 | 状态 |
|------|------|--------|------|------|------|
| **1.1** | Compact Hook 系统 | 🔴 | 高 | 高（扩展性） | ❌ 缺失 |
| **2.1** | Prune 标记机制 | 🟢 | 中 | 中（持久层清理） | ❌ 缺失 |
| **2.2** | 时间触发微压缩 | 🟢 | 低 | 中（cache 感知） | ❌ 缺失 |
| **2.3** | Partial Compact 方向 | 🟢 | 中 | 中（cache 友好） | ❌ 缺失 |
| **2.4** | Prompt 工程升级 | 🟢 | 低 | 中（摘要质量） | ⚠️ 较弱 |
| **2.5** | 精确 Token 计数 | 🟡 | 中 | 高（准确阈值） | ⚠️ 锚点已实现 |
| **2.6** | Cache-sharing fork | 🟡 | 中高 | 高（省 30-50% token） | ❌ 缺失 |
| **3.1** | 文件操作跟踪 | 🔵 | 低 | 中（摘要结构化） | ❌ 缺失 |
| **3.2** | Ghost Snapshot | 🔵 | 中 | 低（undo 保护） | ❌ 缺失 |
| **3.3** | Mid-turn 压缩 | 🔵 | 高 | 低（边界场景） | ❌ 缺失 |
| **3.4** | Context 可视化 | 🔵 | 中 | 低（用户体验） | ❌ 缺失 |
| **3.5** | ~~多模态消息处理~~ | ⏸️ | 低 | 暂缓 | ⏸️ 当前不支持 |
| **3.6** | ~~专用 Compact Agent~~ | ⏸️ | 中 | 暂缓 | ⏸️ 需 agent-tool 重构 |

### 已实现（无需追加）

| 事项 | 实现位置 |
|------|---------|
| 电路熔断 | `ThresholdCompactionPolicy.consecutive_failures` (3次熔断) |
| Post-compact 附件恢复 | `FileAccessTracker` + `recover_file_contents()` |
| Auto-continue Nudge | `AutoContinueNudge` 消息 |
| 锚点 Token 计数 | `TokenUsageTracker.anchored_budget_tokens` |
| 增量重压缩 | `CompactSummary` 消息保留，`compact_input_messages()` 前缀保留 |
| 413 降级重试 | `drop_oldest_turn_group()` 最多3次 |
| 微压缩 | `microcompact.rs`（截断 + 清除） |
| Live tail 录制 | `CompactionTailSnapshot` |
| 三重触发 | Auto / Reactive(413) / Manual |

---

## 四、关键设计决策

### 4.1 从 Claude Code 学到的

1. **Cache-sharing fork**：压缩请求复用主对话 cached prefix，节约 30-50% token
2. **时间触发微压缩**：利用 cache TTL（1h）判断是否清除旧工具结果
3. **Partial compact `from`**：保留前缀方向 cache 友好，适合频繁压缩场景
4. **Post-compact 恢复**：必须恢复文件内容/Plan/Skill/MCP，否则 Agent 失忆
5. **5 层架构**：Auto / Micro / Time / API / Session Memory 分层，各司其职
6. **`<analysis>` scratchpad**：让模型先草拟分析再输出摘要，显著提升质量

### 4.2 从 Codex 学到的

1. **Mid-turn 压缩**：在 turn 执行中实时检测超限并压缩
2. **Ghost Snapshot**：压缩后保留 undo 历史
3. **InitialContextInjection**：Mid-turn 压缩后，在最后用户消息前注入上下文
4. **Remote compact API**：利用 Provider 端的压缩 API（如 OpenAI 的），减少客户端消耗

### 4.3 从 OpenCode 学到的

1. **Plugin 注入**：`Plugin.trigger("experimental.session.compacting")` 可拦截压缩
2. **Prune 策略**：标记不删除，原始内容可审计；保护最近工具结果
3. **Auto-continue**：压缩后自动生成 "Continue" 消息，Agent 无缝继续
4. **三层工作流**：prune → process → create，逻辑清晰

### 4.4 从 pi-mono 学到的

1. **Incremental recompact**：update prompt 模式（旧摘要 + 新内容 → 合并）
2. **Branch summarization**：/tree 导航切换分支时生成分支摘要
3. **File operation tracking**：累积跟踪 read/written/edited，摘要结构化
4. **Session tree**：会话形成树结构，支持原位分支

### 4.5 从 Kimi CLI 学到的

1. **压缩优先级排序**：当前任务 > 错误/修复 > 代码演进 > 系统上下文 > 设计决策 > TODO
2. **简洁参数**：以 `max_preserved_messages` 控制保留范围，参数简单可靠

---

## 五、成功指标

- [ ] 压缩失败率 < 1%（建立基线）
- [ ] 压缩后 Agent 重新访问文件的次数减少 > 50%（Post-compact 恢复的效果）
- [ ] 压缩节省的 token 占总对话 token 的 > 30%
- [ ] 压缩引起的 API 调用浪费 < 0.1%（熔断机制的效果）
- [ ] 压缩后对话可以继续正常的成功率 > 99%
- [ ] Cache-sharing fork 命中缓存率 > 80%（Phase 2）
