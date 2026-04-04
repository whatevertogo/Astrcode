# Compact 功能优化 TODO

> 对比项目：Claude Code、Codex、OpenCode、Kimi CLI、pi-mono
> 基准文档：`docs/architecture/compact-upgrade-plan.md`

---

## 一、现状总览

### Astrcode 当前架构

| 模块 | 文件 | 职责 |
|------|------|------|
| `compaction.rs` | `runtime-agent-loop/src/context_window/compaction.rs` | 完整压缩：前缀/后缀分割 -> LLM 摘要 -> 替换 |
| `microcompact.rs` | `runtime-agent-loop/src/context_window/microcompact.rs` | 微压缩：截断大工具结果 + 清除可清除工具结果 |
| `token_usage.rs` | `runtime-agent-loop/src/context_window/token_usage.rs` | Token 估算：启发式 4 chars/token |
| `compaction_runtime.rs` | `runtime-agent-loop/src/compaction_runtime.rs` | 运行时：Policy/Strategy/Rebuilder 三层 trait |
| `context_pipeline.rs` | `runtime-agent-loop/src/context_pipeline.rs` | 6 阶段上下文管线 |

**三种触发方式**：Auto（阈值）、Reactive（413 错误）、Manual（用户手动）

### 对比矩阵

| 特性 | Claude Code | Codex | OpenCode | Kimi CLI | pi-mono | **Astrcode** |
|------|-------------|-------|----------|----------|---------|-------------|
| 触发机制 | 阈值+反应式+手动+时间微压缩+Session Memory | 阈值+手动+Mid-turn | 阈值+手动 | 手动+阈值 | 阈值+反应式+手动+Extension | 阈值+反应式+手动 |
| 压缩方向 | 全量+Partial(from/up_to) | 后缀保留+Mid-turn | 后缀保留 | 后缀保留 | 后缀保留+Branch摘要 | **仅后缀保留** |
| 微压缩 | 时间触发+Cache-edit+内容清除 | 无 | 工具结果裁剪+Prune | 无 | 无 | 工具结果裁剪+清除 |
| Post-compact 恢复 | 文件重读+Plan+Skill+Agent附件 | Initial context 注入+Ghost snapshots | Auto-continue | 无 | Extension hooks | **无** |
| 多模态处理 | Strip image/document 为占位符 | Strip image | 无 | Strip ThinkPart | 无 | **无** |
| Prompt Cache | Cache-sharing fork+cache_edit | Mid-turn cache 前缀保留 | 无 | 无 | 无 | **无** |
| 熔断机制 | 连续失败3次停试 | 最大重试+backoff | 无 | 无 | 无 | **无** |
| Hook 系统 | Pre/Post/SessionStart | hooks crate (pre/post tool) | Plugin 注入 | 无 | session_before_compact | **无** |
| 增量重压缩 | 无 | CompactSummary 折叠 | 无 | 无 | update prompt 合并 | **无** |
| 远程压缩 | 无 | OpenAI remote compact API | 无 | 无 | 无 | 无 |
| Session Memory | SessionMemory 系统替代部分压缩 | memories 子系统 | 无 | 无 | 无 | 无 |

---

## 二、Phase 1：基础加固

> 目标：补齐缺失的基础能力，降低压缩失败率和信息丢失

### 1.1 Post-compact 附件恢复 [P0]

**问题**：压缩后所有已读文件、Plan 状态全部丢失，Agent 需要"失忆"后重新探索。

**参考实现**：
- Claude Code：`createPostCompactFileAttachments()` 重新读取最近 5 个文件（50K token 预算）、恢复 Plan 文件、Skill 内容、Deferred tools schema、MCP 指令
- Codex：`InitialContextInjection::BeforeLastUserMessage` 在 Mid-turn 压缩后将初始上下文注入到最后的用户消息前
- OpenCode：压缩后自动生成 "Continue" 消息，Agent 无缝继续工作

**数据流**（文件路径提取链路）：

```
AgentLoop step → ToolCall(readFile, {path: "src/main.rs"})
  → StorageEvent::ToolResult { tool_name: "readFile", output: "...", metadata: {path} }
  → FileAccessTracker::record(tool_name, path, timestamp)
  → auto_compact() 触发时：
      FileAccessTracker::recent_files(n=5)
      → 读取文件内容 → PostCompactAttachment::FileContent(path, content)
      → 追加到 compacted_messages() 中摘要之后、后缀之前
```

**实现方案**：
- [ ] 新增 `FileAccessTracker` 结构体（`runtime-agent-loop` 内部），从 `StorageEvent::ToolResult` 中提取 readFile/editFile/writeFile 的路径参数，按时间倒序维护访问记录
- [ ] 在 `CompactResult` 中增加 `post_compact_attachments: Vec<PostCompactAttachment>` 字段
- [ ] 定义 `PostCompactAttachment` 枚举：`FileContent(path, content)` / `PlanState(plan)` / `SkillContent(name, content)` / `McpInstructions`
- [ ] 在 `compaction_runtime.rs` 的 `CompactionRebuilder` trait 中增加 `build_post_compact_attachments()` 方法
- [ ] 压缩后触发 `ContextPipeline` 的重新注入逻辑（system prompt attachments、MCP 指令等）
- [ ] 在 `compacted_messages()` 中将附件追加到摘要消息之后、后缀消息之前
- [ ] 添加 token 预算控制：文件附件总预算 50K tokens，单文件 5K tokens，最多 5 个文件

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/compaction.rs`
- `crates/runtime-agent-loop/src/compaction_runtime.rs`
- `crates/core/src/event/types.rs`
- `crates/core/src/projection/agent_state.rs`

### 1.2 压缩后 Auto-continue [P0]

**问题**：压缩后 Agent 不知道该继续做什么。

**参考实现**：
- OpenCode：`SessionCompaction.create()` 在自动压缩成功后插入合成 "Continue" 消息
- Claude Code：SessionStart hook 在压缩后重新触发，恢复 session 级上下文

**实现方案**：
- [ ] 在 `auto_compact()` 成功后，若触发原因为 `Auto`，在 `compacted_messages()` 中追加一条 `AutoContinueNudge` 消息
- [ ] Nudge 内容参考 OpenCode 格式："The conversation was compacted. Continue from where you left off."
- [ ] 仅 Auto 触发时追加，Manual 触发不追加

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/compaction.rs`

### 1.3 多模态消息处理 [P1]

**问题**：当前压缩时会直接将 image/document 消息发给 LLM 生成摘要，可能因 prompt 过长而失败。

**参考实现**：
- Claude Code：`stripImagesFromMessages()` 将 Image/Document 替换为 `[image: description]` / `[document: filename]`
- Codex：`normalize()` 在 history 归一化时 strip images
- Kimi CLI：在 `prepare()` 中移除 ThinkPart

**实现方案**：
- [ ] 在 `compact_input_messages()` 中增加过滤：检测消息中的多模态内容
- [ ] 将 Image/Document 类型内容替换为 `[image]` / `[document]` 占位文本
- [ ] 保留文本内容不变
- [ ] 与 `LlmMessage` 的多模态扩展同步设计（当前 `LlmMessage::User.content` 是 `String`，扩展为 `Vec<ContentPart>` 时适配）

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/compaction.rs`
- `crates/core/src/action.rs`

### 1.4 熔断机制 [P1]

**问题**：Claude Code 观测到 1,279 个会话出现 50+ 次连续压缩失败，每天浪费约 250K API 调用。Astrcode 当前无熔断保护。

**参考实现**：
- Claude Code：`MAX_CONSECUTIVE_AUTOCOMPACT_FAILURES = 3`，连续失败 3 次后停止自动压缩

**设计决策：熔断放在 Policy 层**

当前 `CompactionRuntime` 已有三层分离：`policy` / `strategy` / `rebuilder`。熔断属于"是否应该压缩"的决策逻辑，应放在 `CompactionPolicy` 层而非 `CompactionRuntime` 层，保持 Runtime 本身无状态。

```
ThresholdCompactionPolicy（现有）
  + consecutive_failures: AtomicUsize
  → should_compact(): 失败 >= 3 时返回 None（仅影响 Auto/Reactive）
  → Manual 不经过 Policy，直接调用 Strategy
```

**实现方案**：
- [ ] 在 `ThresholdCompactionPolicy` 中增加 `consecutive_failures: AtomicUsize` 字段（而非 Runtime 层）
- [ ] 压缩成功时通过 `record_success()` reset 为 0，失败时通过 `record_failure()` +1
- [ ] 在 `should_compact()` 中增加熔断判断：`consecutive_failures >= MAX_CONSECUTIVE_FAILURES` 时返回 `None`
- [ ] 手动压缩（`CompactionReason::Manual`）在 Runtime.compact() 中绕过 Policy 直接调用 Strategy，不受熔断限制
- [ ] 在日志中记录熔断触发事件（`tracing::warn!`）

**涉及文件**：
- `crates/runtime-agent-loop/src/compaction_runtime.rs`

### 1.5 精确 Token 计数（短期） [P1]

**问题**：当前使用 4 chars/token 启发式估算，误差可达正负 30%。

**参考实现**：
- Claude Code：`roughTokenCountEstimation()` + Provider 报告的 usage 数据锚定
- pi-mono：优先使用最后一次 assistant 消息的 usage 数据，回退到估算
- Codex：`approx_token_count()` = `text.len() / 4`

**与现有代码的衔接**：

当前 `TokenUsageTracker` 已有 `anchored_budget_tokens` 锚点机制（优先使用 Provider 报告的 `usage.total_tokens()`）。改进应基于此锚点增强，而非另起炉灶。

```
TokenUsageTracker（现有）
  anchored_budget_tokens: usize  ← Provider 报告的 total_tokens 累积
  + last_input_tokens: Option<usize>  ← Provider 报告的 input_tokens（新增）
  + anchor_timestamp: Option<Instant>  ← 锚点时间（新增）

estimate_request_tokens()（现有）
  → 当有锚点时：anchor_input_tokens + anchor 之后的消息增量估算
  → 当无锚点时：保持现有 4 chars/token 全量估算
```

**实现方案**：
- [ ] 短期：扩展 `TokenUsageTracker::record_usage()` 同时记录 `usage.input_tokens` 作为单次请求锚点
- [ ] 新增 `estimate_request_tokens_anchored()`：当有锚点时，用 `anchor.tokens + 增量消息估算` 替代全量估算；无锚点时回退到现有 `estimate_request_tokens()`
- [ ] 在 `build_prompt_snapshot()` 中优先使用锚点增强估算
- [ ] 中期：为各 Provider 实现 tokenizer（tiktoken for OpenAI-compatible）
- [ ] 长期：将 `estimate_message_tokens` 替换为 Provider-native 精确计数

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/token_usage.rs`
- `crates/runtime-llm/src/lib.rs`

---

## 三、Phase 2：Prompt Cache 优化

> 目标：利用 Prompt Cache 减少压缩后的 API 成本和延迟

### 2.1 Cache-sharing Fork 压缩路径 [P1]

**问题**：压缩时发送独立请求，无法复用主对话的 cached prefix，浪费 token。

**参考实现**：
- Claude Code：fork agent 复用主对话的 cached prefix（system prompt + tools + 前缀消息），节约 30-50% token 成本

**Provider 适配策略**：

不同 Provider 的 prompt cache 机制差异较大，需要分层实现：

```
LlmProvider trait（现有）
  + supports_cache_sharing() -> bool     ← 查询能力（新增）
  + fork_for_compaction() -> Self        ← 创建共享缓存的子请求（新增）

Anthropic 适配：
  → 复用 HTTP 连接 + cache_control: {"type": "ephemeral"} 标记
  → 压缩请求发送相同的 system prompt（带 cache_control）+ 前缀消息
  → 命中 cached prefix，节约 30-50% input tokens

OpenAI 适配：
  → OpenAI 无 Anthropic 式的显式 cache_control
  → v1 仅走独立请求路径（is_cache_sharing_supported = false）
  → 后续可利用 OpenAI automatic cache（相同前缀自动命中）做优化
```

**实现方案**：
- [ ] 在 `LlmProvider` trait 中增加 `supports_cache_sharing() -> bool` 查询方法（默认 `false`）
- [ ] 在 `LlmProvider` trait 中增加 `fork_for_compaction() -> Option<Self>` 方法，返回复用缓存的 Provider 实例
- [ ] 在 `auto_compact()` 中尝试走 cache-sharing 路径：若 Provider 支持，发送相同的 system prompt 和工具定义
- [ ] 如果 cache sharing 失败，回退到当前的独立请求路径
- [ ] Anthropic Provider 实现 `supports_cache_sharing() -> true`，在压缩请求中注入 `cache_control` 标记

**涉及文件**：
- `crates/runtime-llm/src/lib.rs`（`LlmProvider` trait）
- `crates/runtime-agent-loop/src/context_window/compaction.rs`

### 2.2 时间触发微压缩 [P2]

**问题**：当前微压缩仅按 token 压力触发。当 server 端 prompt cache 已过期时，清除旧工具结果不会浪费缓存。

**参考实现**：
- Claude Code：当距上次 assistant 消息的时间间隔超过阈值（30+ 分钟），清除旧工具结果，因为 cache 已过期
- OpenCode：`prune()` 在 session loop 结束后执行，保留最近 40K tokens 的工具输出

**实现方案**：
- [ ] 在 `microcompact.rs` 中增加时间触发逻辑：检查最后 assistant 消息的时间戳
- [ ] 当间隔 > 阈值（可配置，默认 15 分钟）时，清除所有非最近 N 个 compactable 工具的结果
- [ ] 通过配置暴露时间阈值和保留数量
- [ ] 增加 `MicrocompactTrigger` 枚举：`TokenPressure` / `CacheExpired` / `Both`

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/microcompact.rs`
- `crates/runtime-config/src/`

### 2.3 Prune 机制（OpenCode 风格） [P2]

**问题**：微压缩仅在当前消息上操作，不会清理已完成 turn 的旧工具结果。

**参考实现**：
- OpenCode：`prune()` 从后往前扫描，保护最近 40K tokens 的工具输出，超出部分替换为 `[Old tool result content cleared]`，只影响 2+ turn 前的消息

**实现方案**：
- [ ] 在 context pipeline 中增加 `PostTurnPruneStage` 或在 turn 结束时调用 prune
- [ ] 实现 `prune_old_tool_results()` 函数：从后往前累加 token，超出保护区的旧工具结果替换为占位文本
- [ ] 保护最近 N tokens 的工具输出不被 prune
- [ ] 最小 prune 阈值：低于此值不执行 prune

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/microcompact.rs`
- `crates/runtime-agent-loop/src/context_pipeline.rs`

---

## 四、Phase 3：Partial Compact 与增量重压缩

> 目标：支持灵活压缩方向，减少不必要的全量摘要

### 3.1 Partial Compact (from / up_to 方向) [P2]

**问题**：仅支持后缀保留压缩，无法选择压缩方向。

**参考实现**：
- Claude Code：`partialCompactConversation()` 支持用户选择消息节点：
  - `from`：保留前缀（cache 友好），压缩后缀
  - `up_to`：保留后缀（当前行为），压缩前缀
- Codex：`InitialContextInjection::BeforeLastUserMessage` 模式类似 from 方向

**实现方案**：
- [ ] 扩展 `CompactConfig` 增加 `direction: CompactDirection { Suffix, From(usize), UpTo(usize) }`
- [ ] 修改 `split_for_compaction()` 支持按方向分割
- [ ] 在 `compaction_runtime.rs` 增加 `PartialCompactionStrategy`
- [ ] 前端增加消息选择器 UI，允许用户选择压缩范围
- [ ] `from` 方向需处理 prompt cache 兼容性（前缀不变可命中 cache）

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/compaction.rs`
- `crates/runtime-agent-loop/src/compaction_runtime.rs`
- `crates/core/src/action.rs`
- 前端组件

### 3.2 增量重压缩 (Incremental Recompact) [P2]

**问题**：每次压缩都重新摘要完整前缀，浪费 token 且可能丢失信息。

**参考实现**：
- Codex：保留 `CompactSummary` 消息用于增量折叠，检测到 `SUMMARY_PREFIX` 标记
- pi-mono：如果已有摘要，使用 "update" prompt 将新信息合并到现有摘要中
- Astrcode 当前：`compact_input_messages()` 保留之前的 CompactSummary 消息参与压缩

**实现方案**：
- [ ] 修改 `auto_compact()` 入口：检测前缀中是否已有 `CompactSummary` 消息
- [ ] 如果有，只压缩上次摘要之后的新消息
- [ ] 生成增量摘要后，与旧摘要合并（LLM 合并 或 简单拼接）
- [ ] 参考 pi-mono 的 "update" prompt 模式：向 LLM 提供旧摘要 + 新内容，要求合并
- [ ] 避免每次重新摘要整个历史

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/compaction.rs`
- `crates/runtime-agent-loop/src/compaction_runtime.rs`

### 3.3 文件跟踪（pi-mono 风格） [P3]

**问题**：摘要中缺乏对读写文件的结构化跟踪，压缩后文件上下文丢失严重。

**参考实现**：
- pi-mono：`extractFileOperations()` 累积跟踪 read/written/edited 文件，在摘要末尾追加 `<read-files>` 和 `<modified-files>` XML 段

**实现方案**：
- [ ] 在压缩流程中提取工具调用中的文件操作（readFile, editFile, writeFile 的路径参数）
- [ ] 累积维护 `FileOperationSet { read, written, edited }` 跨多次压缩
- [ ] 在摘要 prompt 中注入文件操作信息
- [ ] 在摘要输出中增加文件列表段

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/compaction.rs`

---

## 五、Phase 4：智能化

> 目标：通过 Hook 系统和更精细的 prompt 工程提升摘要质量

### 4.1 Pre/Post Compact Hook 系统 [P2]

**问题**：无插件可介入压缩流程的扩展点。

**参考实现**：
- Claude Code：`PreCompact` / `PostCompact` / `SessionStart` 三类 hook
- OpenCode：`experimental.session.compacting` plugin hook
- pi-mono：`session_before_compact` event（可取消或提供自定义摘要）

**实现方案**：
- [ ] 在 `core` 中定义 `CompactHookEvent` 和 `CompactHookResult` 类型
- [ ] 定义 hook 类型：`PreCompact`（可修改 prompt）、`PostCompact`（可执行恢复）、`SessionRestore`（压缩后恢复 session 上下文）
- [ ] 在 `compaction_runtime.rs` 的 `compact()` 前后调用 hook
- [ ] 允许插件注入自定义压缩指令
- [ ] 参考现有 `PolicyHook` 的注册和链式执行机制
- [ ] 复用 `crates/plugin/` 的 JSON-RPC 通信通道

**涉及文件**：
- `crates/core/src/`（hook 事件类型）
- `crates/runtime-agent-loop/src/compaction_runtime.rs`
- `crates/plugin/` / `crates/sdk/`

### 4.2 摘要 Prompt 工程升级 [P2]

**问题**：摘要质量可以进一步提升。

**参考实现**：
- Claude Code：`<analysis>` scratchpad 机制（模型先草拟分析再输出摘要）+ NO_TOOLS 约束 + `formatCompactSummary()` 标签清理
- Kimi CLI：压缩优先级排序（当前任务 > 错误修复 > 代码演进 > 系统上下文 > 设计决策 > TODO）
- OpenCode：plugin 可定制 prompt
- pi-mono：结构化 markdown（Goal / Progress / Decisions / Next Steps）

**与现有代码的关系**：

当前 `build_compact_system_prompt()` 已经要求模型返回 `<analysis>` + `<summary>` 双 XML 块，`extract_summary()` 也已正确提取 `<summary>` 内容。因此"analysis scratchpad"本身已实现，改进重点在于：
1. **确保 analysis 被正确消费**：当前 `extract_summary()` 跳过了 `<analysis>` 内容，但不验证 analysis 是否存在。应增加校验：若 LLM 未返回 analysis 块则降级处理。
2. **NO_TOOLS 约束**：当前 system prompt 已有 "Never call tools" 指令，但可加强为更显眼的约束（如放在 prompt 最前面）。
3. **标签清理**：当前 `extract_summary()` 仅做基本提取，未做 Claude Code 风格的 `formatCompactSummary()` 清理（去除残留标签、多余空白）。

**实现方案**：
- [ ] 改进 `extract_summary()`：增加 `<analysis>` 块存在性校验，缺失时记录 warning 但不阻断压缩
- [ ] 改进 `build_compact_system_prompt()`：将 NO_TOOLS 约束移到 prompt 最前面，使用更强硬的措辞
- [ ] 新增 `format_compact_summary()` 函数：做标签清理（去除残留 XML 标签、规范化空白）
- [ ] 支持通过 Hook 注入自定义指令
- [ ] 增加优先级指导：参考 Kimi CLI 的内容优先级排序（当前任务 > 错误修复 > 代码演进 > 系统上下文 > 设计决策 > TODO）

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/compaction.rs`（`build_compact_system_prompt`、`extract_summary`）

### 4.3 Context Usage 可视化 [P3]

**问题**：用户无法直观了解上下文使用情况。

**参考实现**：
- Claude Code：`ContextVisualization` 组件展示 token 按类别分布、`/context` 命令
- OpenCode：status bar 显示 context usage 百分比

**实现方案**：
- [ ] 增加 `/context` 命令：显示当前上下文使用量、按类别分布（system prompt / tools / conversation / tool results）
- [ ] 前端增加 context usage 指示器
- [ ] 在压缩前显示 token 节省预估

**涉及文件**：
- 新增 command 文件
- 前端组件
- `crates/runtime-agent-loop/src/context_window/token_usage.rs`

### 4.4 Mid-turn 压缩 [P3]

**问题**：当前压缩仅在 turn 间触发，无法在 turn 执行过程中处理 context 超限。

**参考实现**：
- Codex：Mid-turn 压缩，在 model response 完成后检测 token 超限，inline 触发压缩
- Claude Code：Reactive compact 在 413 错误时触发

**实现方案**：
- [ ] 在 `turn_runner` 的 step loop 中，每次 LLM 响应后检测 token 使用量
- [ ] 如果超过阈值，在 step 间触发压缩（而非等 turn 结束）
- [ ] 参考 Codex 的 `InitialContextInjection::BeforeLastUserMessage` 模式
- [ ] 确保压缩后工具调用上下文不丢失

**与熔断机制的交互**：
- Mid-turn 压缩触发时使用 `CompactionReason::Reactive`，经过 `CompactionPolicy.should_compact()` 判断
- 若熔断器已触发（`consecutive_failures >= 3`），Mid-turn 自动压缩同样被跳过
- 跳过时记录 warning 日志，Agent 应向用户报告 context 超限但无法自动压缩
- 手动触发的 Mid-turn 压缩（如有）不受熔断限制

**涉及文件**：
- `crates/runtime-agent-loop/src/agent_loop/turn_runner.rs`
- `crates/runtime-agent-loop/src/compaction_runtime.rs`

---

## 六、实施优先级总览

| 优先级 | 编号 | 改进项 | 参考来源 | 影响范围 | 估算工时 |
|--------|------|--------|----------|----------|----------|
| **P0** | 1.1 | Post-compact 附件恢复 | Claude Code, Codex | 压缩后 Agent 不失忆 | 3d |
| **P0** | 1.2 | 压缩后 Auto-continue | OpenCode | 压缩后无缝继续 | 0.5d |
| **P1** | 1.3 | 多模态消息处理 | Claude Code, Codex, Kimi CLI | 压缩可靠性 | 2d |
| **P1** | 1.4 | 熔断机制 | Claude Code | 防止 API 调用浪费 | 1d |
| **P1** | 1.5 | 精确 Token 计数 | pi-mono, Claude Code | 阈值判断准确性 | 2d（短期）+ 5d（中期） |
| **P1** | 2.1 | Cache-sharing Fork | Claude Code | API 成本降低 | 3d |
| **P2** | 2.2 | 时间触发微压缩 | Claude Code | Cache 利用优化 | 1.5d |
| **P2** | 2.3 | Prune 机制 | OpenCode | 工具结果清理 | 2d |
| **P2** | 3.1 | Partial Compact | Claude Code, Codex | 压缩灵活性 | 5d（含前端 UI 2d） |
| **P2** | 3.2 | 增量重压缩 | Codex, pi-mono | 减少重复摘要 | 3d |
| **P2** | 4.1 | Pre/Post Compact Hook | Claude Code, OpenCode, pi-mono | 插件扩展性 | 3d |
| **P2** | 4.2 | 摘要 Prompt 升级 | Claude Code, Kimi CLI, pi-mono | 摘要质量 | 2d |
| **P3** | 3.3 | 文件跟踪 | pi-mono | 摘要信息完整性 | 1.5d |
| **P3** | 4.3 | Context Usage 可视化 | Claude Code, OpenCode | 用户体验 | 3d（含前端 UI 1.5d） |
| **P3** | 4.4 | Mid-turn 压缩 | Codex | 极端场景兜底 | 3d |

### 建议实施顺序

```
Phase 1 (P0+P1): 1.1(3d) -> 1.2(0.5d) -> 1.4(1d) -> 1.3(2d) -> 1.5(2d)  ≈ 8.5d
Phase 2 (P1+P2): 2.1(3d) -> 2.2(1.5d) -> 2.3(2d)                        ≈ 6.5d
Phase 3 (P2):    3.2(3d) -> 3.1(5d) -> 4.1(3d) -> 4.2(2d)                ≈ 13d
Phase 4 (P3):    3.3(1.5d) -> 4.3(3d) -> 4.4(3d)                          ≈ 7.5d
                                                              总计 ≈ 35.5d（7 人周）
```

---

## 七、关键设计决策参考

### 从 Claude Code 学到的

1. **Cache-sharing fork**：压缩请求复用主对话的 cached prefix，节约 30-50% token 成本
2. **时间触发微压缩**：利用 cache TTL 判断清除旧工具结果是否浪费 cache
3. **Partial compact**：`from` 方向保持前缀 cache，`up_to` 方向保持后缀上下文
4. **Post-compact 恢复**：必须恢复文件内容、Plan、Skill、Deferred tools
5. **熔断器**：3 次连续失败后停止自动压缩，手动不受限
6. **Session Memory Compaction**：用 session memory 替代完整 API 压缩，更轻量

### 从 Codex 学到的

1. **Mid-turn 压缩**：turn 执行过程中实时压缩（`InitialContextInjection::BeforeLastUserMessage`）
2. **Remote compact API**：支持调用 Provider 端压缩 API（如 OpenAI），减少客户端 token 消耗
3. **用户消息保留**：压缩后保留真实用户消息（20K token 预算），保持用户意图可见
4. **Ghost snapshots**：压缩后 `/undo` 仍然可用
5. **Startup context**：分 section 预算构建启动上下文（Current Thread / Recent Work / Workspace Map）

### 从 OpenCode 学到的

1. **Plugin 注入**：允许第三方插件在压缩时注入上下文或替换 prompt
2. **Prune 策略**：从后往前扫描，保护最近 N tokens，清除超出预算的旧结果
3. **Auto-continue**：压缩后自动生成 "Continue" 消息
4. **Compaction Agent**：独立隐藏 Agent 执行压缩，所有工具权限 deny

### 从 Kimi CLI 学到的

1. **压缩优先级排序**：当前任务 > 错误修复 > 代码演进 > 系统上下文 > 设计决策 > TODO
2. **Protocol-based 策略**：`Compaction` trait 允许灵活替换压缩算法
3. **简洁的 `max_preserved_messages` 参数**：以消息数而非 token 数控制保留范围

### 从 pi-mono 学到的

1. **增量摘要合并**：如果已有摘要，使用 "update" prompt 将新信息合并到现有摘要
2. **累积文件跟踪**：跨多次压缩跟踪 read/written/edited 文件
3. **Extension hooks**：`session_before_compact` 允许扩展取消或自定义压缩
4. **Split-turn 处理**：单个 turn 超过保留预算时，分割 turn 并行生成两个摘要再合并
5. **Branch summarization**：会话分支切换时的摘要机制

---

## 八、风险和注意事项

1. **向后兼容**：`CompactTrigger`、`StorageEvent::CompactApplied` 等类型变更需同步到 JSONL 存储层
2. **Provider 差异**：不同 LLM Provider 的 cache 机制不同，cache-sharing 需按 Provider 适配
3. **多模态扩展**：`LlmMessage::User.content` 从 `String` 扩展为 `Vec<ContentPart>` 是大范围变更，需协调
4. **增量重压缩质量**：合并两次摘要可能丢失中间细节，需实测决定 LLM 合并还是简单拼接
5. **Partial compact cache 一致性**：`from` 方向保持 cache 前缀，`up_to` 方向会破坏 cache，需在 UI 中提示
6. **Post-compact 文件读取成本**：重读文件会增加 API token 消耗，需预算控制
7. **Mid-turn 压缩的复杂性**：在工具调用链中间压缩可能导致状态不一致，需仔细设计

### 回滚与降级策略

| 改进项 | Feature Flag | 降级行为 | 回滚方式 |
|--------|-------------|----------|----------|
| 1.1 Post-compact 附件恢复 | `compact.post_compact_recovery` | 关闭后不恢复附件，保持当前行为 | 配置项默认 `false`，验证稳定后开启 |
| 1.4 熔断机制 | `compact.circuit_breaker` | 关闭后不限制失败次数 | `MAX_CONSECUTIVE_FAILURES` 可配置为 `0` 禁用 |
| 2.1 Cache-sharing Fork | `compact.cache_sharing` | 关闭后走独立请求路径 | `supports_cache_sharing()` 返回 `false` 即回退 |
| 2.2 时间触发微压缩 | `compact.time_triggered_micro` | 关闭后仅 token 压力触发 | 阈值设为 `0` 等效禁用 |
| 3.1 Partial Compact | `compact.partial_direction` | 关闭后仅后缀保留 | 前端隐藏方向选择器 |
| 3.2 增量重压缩 | `compact.incremental_recompact` | 关闭后全量重摘要 | 检测到摘要时不走增量路径 |
| 4.1 Hook 系统 | `compact.hooks` | 关闭后不调用 hook | 空 hook 注册表 |
| 4.4 Mid-turn 压缩 | `compact.mid_turn` | 关闭后仅 turn 间压缩 | 在 step loop 中跳过检测 |

**Feature flag 配置方式**：通过 `runtime-config` 的 `CompactionConfig` 暴露布尔开关，所有新功能默认关闭，逐项验证后开启。可在 `runtime-config/src/types.rs` 中统一管理。

---

## 九、测试计划

每个 Phase 完成后需通过的测试：

### Phase 1 测试
- [ ] Post-compact 后 Agent 能引用之前读过的文件内容
- [ ] `FileAccessTracker` 正确从 `StorageEvent::ToolResult` 中提取文件路径
- [ ] 文件附件不超过 50K token 总预算，单文件不超过 5K token
- [ ] Auto-continue nudge 在 Auto 触发时出现，Manual 时不出现
- [ ] 包含图片消息的压缩不失败
- [ ] 熔断触发后自动压缩停止，手动压缩仍可用
- [ ] Token 估算与 Provider 报告值的偏差：ASCII 内容 < 10%，CJK 内容 < 20%
- [ ] 锚点增强估算在有 Provider usage 数据时优于纯启发式估算

### Phase 2 测试
- [ ] Cache-sharing 路径在 Anthropic Provider 上生效，OpenAI Provider 走独立请求
- [ ] Cache-sharing 失败时回退到独立请求
- [ ] 时间触发微压缩在 idle 后正确清除旧工具结果
- [ ] Prune 保留最近 N tokens 的工具输出

### Phase 3 测试
- [ ] `from` 方向压缩后前缀消息不变（cache 友好）
- [ ] `up_to` 方向压缩后后缀消息不变
- [ ] 增量重压缩只处理新消息，不重复摘要旧内容
- [ ] 文件跟踪在多次压缩后正确累积

### Phase 4 测试
- [ ] Pre-compact hook 可修改压缩 prompt
- [ ] Post-compact hook 可执行恢复操作
- [ ] 摘要包含 analysis 块但不输出给后续对话
- [ ] `/context` 命令正确显示各类别 token 分布
- [ ] Mid-turn 压缩后工具调用链可继续执行
- [ ] Mid-turn 压缩失败计入熔断计数，熔断触发后 Mid-turn 压缩被跳过

### 性能回归测试（跨 Phase）
- [ ] 压缩操作延迟上限：单次 `auto_compact()` 完成时间 < 10s（含 LLM 调用）
- [ ] 微压缩操作延迟上限：单次 `apply_microcompact()` 完成时间 < 50ms（纯本地操作）
- [ ] 内存占用：压缩流程峰值内存增量 < 5MB
- [ ] Token 估算性能：`estimate_request_tokens()` 对 1000 条消息的估算时间 < 10ms
- [ ] 并发安全：`CompactionPolicy` 的 `consecutive_failures` 在多线程环境下正确计数
