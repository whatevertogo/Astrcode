# Compact 系统优化计划

> 基于对五个主流 coding agent（Claude Code, Codex, OpenCode, Kimi CLI, pi-mono）的深度对比分析
> 结合 Astrcode 现有架构代码审查结果，进行优先级排序

---

## 一、对比分析总览

### 1.1 各方案核心优势

| 项目 | 核心优势 | 值得借鉴的点 |
|------|---------|-------------|
| **Claude Code** | 工业级稳定性 | 5 层架构（Auto/Micro/Time/API/Session Memory）、电路熔断、Post-compact 附件恢复（文件+Plan+Skill+MCP）、Cache-sharing fork、时间微压缩（cache TTL 感知） |
| **Codex** | 灵活的压缩方向 | Mid-turn 压缩、InitialContextInjection（前缀/后缀注入）、OpenAI remote compact API、Ghost Snapshot 保持 /undo 可用、Prefix caching / Context Edit |
| **OpenCode** | 三层渐进压缩 | prune→process→create 三层工作流、专用 compact agent（无工具权限）、Auto-continue 合成消息、Prune 标记机制（可审计） |
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

### 1.2 缺失能力清单

| 缺失项 | 影响 | 参考项目 | 优先级 |
|--------|------|---------|--------|
| Compact Hook 系统 | 🔴 高 — 插件无法介入压缩流程 | Claude Code, pi-mono | 🔴 Phase 1 |
| Cache-sharing fork | 🟡 中 — 压缩无法复用主对话 cache | Claude Code | 🟡 Phase 2 |
| 时间触发微压缩 | 🟢 低 — cache 过期时不清理旧工具结果 | Claude Code | 🟢 Phase 2 |
| Prune 标记机制 | 🟡 中 — 事件直接丢弃，不可审计 | OpenCode | 🟢 Phase 2 |
| Partial Compact 方向 | 🟢 低 — 只支持后缀保留 | Claude Code, Codex | 🟢 Phase 2 |
| Prompt 工程升级 | 🟢 低 — 缺少 labels 清理/analysis 校验/优先级 | Claude Code, Kimi CLI | 🟢 Phase 2 |
| 精确 Token 计数 | 🟡 中 — 4 chars/token 启发式 + 锚点 | 全部项目 | 🟡 Phase 2 |
| 文件操作跟踪 | 🟢 低 — 摘要中无结构化文件读写信息 | pi-mono | 🔵 Phase 3 |
| Ghost Snapshot | 🔵 低 — 压缩后 /undo 可能受影响 | Codex | 🔵 Phase 3 |
| Context Usage 可视化 | 🔵 低 — 前端无 token 分布指示 | Claude Code, OpenCode | 🔵 Phase 3 |

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

pub enum CompactPhase {
    PreCompact,    // 压缩前，可修改 prompt/messages/tools，可取消
    PostCompact,   // 压缩后，可执行恢复操作
}

#[async_trait]
pub trait CompactHook: Send + Sync {
    fn phase(&self) -> CompactPhase;
    async fn on_event(&self, event: &CompactHookEvent<'_>) -> Result<CompactHookResult>;
}
```

**实现要点**：
- 复用 `crates/plugin/` 的 JSON-RPC 通信通道
- PreCompact hook 支持取消（`cancel: true`）
- PostCompact hook 可请求读取文件恢复附件

**涉及文件**：
- `crates/core/src/`（compact hook 事件类型）
- `crates/runtime-agent-loop/src/compaction_runtime.rs`
- `crates/plugin/`（hook 注册和通信）

---

### Phase 2：Cache 优化 & 压缩方向 & Prompt 工程

> **目标**：利用 Prompt Cache 减少成本和延迟；支持灵活压缩方向；提升摘要质量

#### 2.1 Cache-sharing Fork 压缩

**Codex Prefix Caching / Context Edit**：
Codex 支持 `prefix_caching`（压缩时从头删除消息保持 KV cache 有效）和 `context_edit`（Anthropic 原生 API，通过 Context Edit API 直接编辑上下文，不需要重新发送前缀消息）。

```rust
// LlmProvider trait 扩展
pub trait LlmProvider: Send + Sync {
    fn supports_cache_sharing(&self) -> bool { false }
    fn fork_for_compaction(&self) -> Option<Self> { None }
    // Codex 风格
    fn supports_prefix_caching(&self) -> bool { false }
    fn supports_context_edit(&self) -> bool { false }
}
```

- **Anthropic**：`supports_cache_sharing() -> true`（cache_control: ephemeral），`supports_context_edit() -> true`（Anthropic v1 messages context edit API）
- **OpenAI v1**：`supports_prefix_caching() -> true`（相同前缀自动命中），`supports_cache_sharing() -> false`
- **OpenAI Context Edit API**：`supports_context_edit() -> true`，通过 Context Edit API 直接编辑上下文，不需要重新发送前缀消息

**涉及文件**：`runtime-llm/src/lib.rs`、`compaction.rs`、`anthropic.rs`

---

#### 2.2 时间触发微压缩

**Claude Code**：
```rust
pub enum MicrocompactTrigger { TokenPressure, CacheExpired, Both }

fn should_cache_expire_compact(
    last_assistant_ts: DateTime<Utc>, now: DateTime<Utc>,
    threshold_seconds: u64,  // 默认 900 (15分钟)
) -> bool { (now - last_assistant_ts).num_seconds() > threshold_seconds as i64 }
```

**涉及文件**：`microcompact.rs`、`runtime-config`

---

#### 2.3 Prune 标记机制（OpenCode 风格）

OpenCode 的 prune 从后往前扫描，保护最近 40K tokens 工具输出，超出替换占位；标记不删除（可审计）。

**OpenCode 设计**：
```rust
pub struct PostTurnPruneConfig {
    pub prune_protect_tokens: usize,   // 默认 40K
    pub prune_minimum: usize,          // 默认 20K
    pub protected_tools: HashSet<String>,
}

fn prune_old_tool_results(events: &[StoredEvent], config: &PostTurnPruneConfig) -> Vec<StoredEvent>;
```

**涉及文件**：`context_pipeline.rs`、新增 `prune.rs`、`storage`

---

#### 2.4 Partial Compact 方向

**Codex 风格**：`from` 方向保留前缀，压缩后缀（cache 友好）；`up_to` 方向保留后缀，压缩前缀（当前默认）。

```rust
pub enum CompactDirection {
    From { from_index: usize },  // 保留前缀，压缩后缀（cache 友好）
    UpTo { up_to_index: usize }, // 保留后缀，压缩前缀（当前默认）
}
```

**涉及文件**：`compaction.rs`、`compaction_runtime.rs`、`action.rs`

---

#### 2.5 Prompt 工程升级

**问题**：当前 `build_compact_system_prompt` 已有 9 段结构，但缺少 `<analysis>` 校验、标签清理、内容优先级、NO_TOOLS 强化。

**Claude Code 参考**：Analysis scratchpad + formatCompactSummary() 标签清理。
**Kimi CLI 参考**：压缩内容优先级排序。

**设计方案**：
```rust
// 1. extract_summary() 增加 analysis 块校验
fn extract_summary(response: &str) -> Result<String> {
    if !response.contains("<analysis>") {
        log::warn!("compact: missing <analysis> block — summary quality may be low");
    }
    // ... 提取 <summary> 内容
}

// 2. format_compact_summary() 新增标签清理 + 空白规范化
pub fn format_compact_summary(summary: &str) -> String {
    let cleaned = summary
        .replace("<summary>", "")
        .replace("</summary>", "")
        .replace("<analysis>", "")
        .replace("</analysis>", "")
        .trim();
    let normalized = regex::Regex::new(r"\s+").unwrap().replace_all(cleaned, " ");
    format!("[Auto-compact summary]\n{}\n\nContinue from this summary.", normalized.trim())
}

// 3. build_compact_system_prompt() 强化:
//    - CRITICAL RULE 放最前，大写 NO_TOOLS
//    - 6 级内容优先级（当前任务 > 用户消息 > 错误修复 > ...）
//    - 分析自检块
//    - Output ONLY 约束
//    - 支持通过 Hook 注入自定义指令
```

**完整 prompt 见 [`compact-prompt-engineering.md`](compact-prompt-engineering.md)。**

**涉及文件**：`compaction.rs`、`agent_state.rs`

---

#### 2.6 精确 Token 计数

```rust
pub struct TokenUsageTracker {
    anchored_budget_tokens: usize,
    last_input_tokens: Option<usize>,        // 新增
    anchor_message_index: Option<usize>,     // 新增
    anchor_timestamp: Option<Instant>,       // 新增
}

pub fn estimate_context_tokens(&self) -> usize;  // 锚点增强估算
```

**中期**：集成 `tiktoken`（OpenAI）+ 复用 Anthropic token 返回。

**涉及文件**：`token_usage.rs`、`runtime-llm/src/lib.rs`

---

### Phase 3：智能化（高级场景覆盖）

> **目标**：提升摘要质量、扩展性、可审计性

#### 3.1 文件操作跟踪

```rust
pub struct FileOperationSet { read: Vec<PathBuf>, written: Vec<PathBuf>, edited: Vec<PathBuf> }
// 在摘要 prompt 中注入文件信息，输出追加 <read-files> / <modified-files> XML 段
```

**涉及文件**：`compaction.rs`、`compaction_runtime.rs`

---

#### 3.2 Ghost Snapshot（Undo 保护）

```rust
pub enum StorageEvent {
    // ... 现有变体
    GhostSnapshot { original_events: Vec<StoredEvent>, compact_seq_range: (u64, u64) },
}
```

- 不参与对话上下文投影（`project()` 中 skip）
- 存储开销：一次 compact 约额外 5-10K tokens

**涉及文件**：`event/types.rs`、`session.rs`、`compaction.rs`

---

#### 3.3 Mid-turn 压缩

```rust
async fn step(&mut self) -> Result<StepResult> {
    let output = self.provider.generate(request, sink).await?;
    if self.token_tracker.estimate_context_tokens() >= threshold {
        self.compact_runtime.compact_mid_turn(self.tail_snapshot.clone())?;
    }
}
```

**涉及文件**：`agent_loop.rs`、`compaction_runtime.rs`

---

#### 3.4 Context Usage 可视化

```
Tauri command: get_context_usage() → ContextUsage { total_tokens, context_window, usage_percent, breakdown: { system_prompt, tools, history, files } }
```

**涉及文件**：`commands.rs`、`frontend/components`、`server`

---

#### 3.5 Split Turn 处理（pi-mono）

当单个 turn 超过预算时，生成两个摘要（历史摘要 + 前缀摘要）并合并，而非截断整个 turn。

**pi-mono 参考**：`splitTurnWhenBudgetExceeded()` 将单个大 turn 分割为两部分，分别生成摘要后合并。

---

#### 3.6 Token Budget 分层保护（Claude Code 多层缓冲区）

Claude Code 使用 5 层缓冲区设计（13K/20K/20K/3K），根据紧急程度分层处理：
- **13K**：极端紧急，立即压缩
- **20K**：高优先级，主动压缩
- **20K**：正常压缩
- **3K**：预留 buffer，防止超限

```rust
pub struct MultiLevelBuffer {
    pub extreme_threshold: usize,    // 13K
    pub high_threshold: usize,       // 20K
    pub normal_threshold: usize,     // 20K
    pub reserve_buffer: usize,       // 3K
}
```

---

#### 3.7 Session Memory 系统（Claude Code）

SessionMemory 系统替代部分压缩，将长期记忆存储到 SessionMemory 系统，减少 token 消耗。

```rust
pub struct SessionMemory {
    pub persistent_facts: Vec<String>,    // 长期事实
    pub project_context: String,          // 项目上下文
    pub user_preferences: String,         // 用户偏好
    pub conversation_summary: String,     // 对话摘要
}
```

---

## 三、暂缓项（需前置重构）

> 以下事项依赖其他模块的大范围重构，暂不纳入近期实施计划。

### T.1 多模态消息处理（依赖 `ContentPart` 扩展）

> ⏸️ 当前项目不支持多模态，仅预留设计。

```rust
fn strip_multimodal_for_compact(content: &ContentPart) -> String {
    match content {
        ContentPart::Text(t) => t.clone(),
        ContentPart::Image(_) => "[image]".to_string(),
        ContentPart::Document(_) => "[document]".to_string(),
    }
}
```

**涉及文件**：`compaction.rs`、`action.rs`

---

### T.2 专用 Compact Agent（依赖 agent-tool 重构）

> ⏸️ 需 agent-tool 重构后实现。

OpenCode 参考：专用 compact agent，无工具权限，可配置低成本模型（如 claude-haiku）。等 agent-tool 重构完成后，复用其 Agent 抽象创建 compact-only 实例。

---

## 四、实施优先级总览

| # | 事项 | Phase | 优先级 | 难度 | 收益 | 状态 |
|---|------|-------|--------|------|------|------|
| 1 | Compact Hook 系统 | 1 | 🔴 | 高 | 高（扩展性） | ❌ 缺失 |
| 2 | Cache-sharing fork | 2 | 🟡 | 中高 | 高（省 30-50% token） | ❌ 缺失 |
| 3 | 时间触发微压缩 | 2 | 🟢 | 低 | 中（cache 感知） | ❌ 缺失 |
| 4 | Prune 标记机制 | 2 | 🟢 | 中 | 中（持久层清理） | ❌ 缺失 |
| 5 | Partial Compact 方向 | 2 | 🟢 | 中 | 中（cache 友好） | ❌ 缺失 |
| 6 | Prompt 工程升级 | 2 | 🟢 | 低 | 中（摘要质量） | ⚠️ 较弱 |
| 7 | 精确 Token 计数 | 2 | 🟡 | 中 | 高（准确阈值） | ⚠️ 锚点已实现 |
| 8 | 文件操作跟踪 | 3 | 🔵 | 低 | 中（摘要结构化） | ❌ 缺失 |
| 9 | Ghost Snapshot | 3 | 🔵 | 中 | 低（undo 保护） | ❌ 缺失 |
| 10 | Mid-turn 压缩 | 3 | 🔵 | 高 | 低（边界场景） | ❌ 缺失 |
| 11 | Context 可视化 | 3 | 🔵 | 中 | 低（用户体验） | ❌ 缺失 |
| 12 | Split Turn 处理 | 3 | 🔵 | 低 | 中（转场处理） | ❌ 缺失 |
| 13 | 多层 Buffer | 3 | 🔵 | 低 | 中（精细控制） | ❌ 缺失 |
| 14 | Session Memory | 3 | 🔵 | 中 | 低（长期记忆） | ❌ 缺失 |
| T1 | ~~多模态消息处理~~ | — | ⏸️ | 低 | 暂缓 | 需 ContentPart 扩展 |
| T2 | ~~专用 Compact Agent~~ | — | ⏸️ | 中 | 暂缓 | 需 agent-tool 重构 |

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

## 五、关键设计决策

### 5.1 从 Claude Code 学到的

1. **Cache-sharing fork**：压缩请求复用 cached prefix，节约 30-50% token
2. **时间触发微压缩**：利用 cache TTL（1h）判断清除旧工具结果
3. **Partial compact `from`**：保留前缀方向 cache 友好
4. **Post-compact 恢复**：必须恢复文件/Plan/Skill/MCP，否则 Agent 失忆
5. **5 层架构**：Auto / Micro / Time / API / Session Memory 分层
6. **`<analysis>` scratchpad**：让模型先草拟分析再输出摘要
7. **多层 Buffer 设计**：根据紧急程度分层处理（13K/20K/20K/3K）
8. **Session Memory 系统**：替代部分压缩，长期记忆独立存储

### 5.2 从 Codex 学到的

1. **Mid-turn 压缩**：在 turn 执行中实时检测超限并压缩
2. **Ghost Snapshot**：压缩后保留 undo 历史
3. **InitialContextInjection**：Mid-turn 压缩后，在最后用户消息前注入上下文
4. **Remote compact API**：利用 Provider 端的压缩 API
5. **前缀缓存策略**：从头删除而非从尾删除，保持 KV cache 前缀有效
6. **Prefix Caching / Context Edit**：Anthropic Context Edit API 直接修改上下文
7. **POSIX 原子写入**：JSONL 紧凑日志用原子写入（防止并发问题）
8. **循环重试 + 指数退避**：压缩失败时重试，指数退避策略
9. **用户转义边界截断**：复杂的分叉追踪，避免截断用户消息

### 5.3 从 OpenCode 学到的

1. **Plugin 注入**：`Plugin.trigger("experimental.session.compacting")` 可拦截压缩
2. **Prune 策略**：标记不删除，原始内容可审计；保护最近工具结果
3. **Auto-continue**：压缩后自动生成 "Continue" 消息
4. **三层工作流**：prune → process → create
5. **配置 JSON 覆盖**：`config.compaction.auto/prune/reserved` 灵活可调
6. **合成消息标记**：`synthetic: true` 标记自动生成的消息

### 5.4 从 pi-mono 学到的

1. **Incremental recompact**：update prompt 模式（旧摘要 + 新内容 → 合并）
2. **Branch summarization**：/tree 导航切换分支时生成分支摘要
3. **File operation tracking**：累积跟踪 read/written/edited
4. **Session tree**：会话形成树结构，支持原位分支
5. **Split Turn 处理**：当单个 turn 超限，生成两个摘要（历史 + 前缀）
6. **嵌套紧凑中文件操作不覆盖累积**：文件路径追踪支持继承继承

### 5.5 从 Kimi CLI 学到的

1. **压缩优先级排序**：当前任务 > 错误/修复 > 代码演进 > 系统上下文 > 设计决策 > TODO
2. **简洁参数**：以 `max_preserved_messages` 控制保留范围
3. **动态元数据刷新**：压缩时从 wire 文件读取最新状态

---

## 六、成功指标

- [ ] 压缩失败率 < 1%（建立基线）
- [ ] 压缩后 Agent 重新访问文件的次数减少 > 50%
- [ ] 压缩节省的 token 占总对话 token 的 > 30%
- [ ] 压缩引起的 API 调用浪费 < 0.1%
- [ ] 压缩后对话可以继续正常的成功率 > 99%
- [ ] Cache-sharing fork 命中缓存率 > 80%
