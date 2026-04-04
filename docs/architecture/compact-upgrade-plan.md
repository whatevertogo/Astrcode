# Compact 功能升级计划

> 基于对 Claude Code、Codex、OpenCode、Kimi CLI 的 compact 实现的对比分析，
> 为 Astrcode 制定分阶段升级路线。

## 1. 现状分析

### 1.1 Astrcode 当前架构

| 模块 | 文件 | 职责 |
|------|------|------|
| `compaction.rs` | 完整压缩 | 前缀/后缀分割 → LLM 摘要 → 替换 |
| `microcompact.rs` | 微压缩 | 截断大工具结果 + 清除可清除工具结果 |
| `token_usage.rs` | Token 估算 | 启发式 4 chars/token，阈值计算 |
| `compaction_runtime.rs` | 运行时 | Policy/Strategy/Rebuilder 三层分离 |

**三种触发方式**：Auto（阈值）、Reactive（413 错误）、Manual（用户手动）

**已知限制（代码中 TODO 标注）**：
- 仅支持文本消息，多模态需额外处理
- 仅支持后缀保留，不支持 Claude 风格的 "from" 方向部分压缩
- 压缩后不恢复 prompt 附件
- 总是重新摘要完整前缀，无增量重压缩
- Token 估算为纯启发式，无 Provider 精确计数

### 1.2 各项目对比矩阵

| 特性 | Claude Code | Codex | OpenCode | Kimi CLI | **Astrcode** |
|------|-------------|-------|----------|----------|-------------|
| **触发机制** | 阈值 + 反应式 + 手动 + 时间微压缩 | 阈值 + 手动 + Mid-turn 压缩 | 阈值 + 手动 | 手动 | 阈值 + 反应式 + 手动 |
| **压缩方向** | 全量 + Partial (`from`/`up_to`) | 后缀保留 + Mid-turn | 后缀保留 | 后缀保留 | **仅后缀保留** |
| **摘要 Prompt** | 9 段结构化（含 analysis scratchpad） | 外部模板文件 | 8 段结构化 | XML 结构化 | 9 段结构化 |
| **微压缩** | 时间触发 + Cache-edit + 内容清除 | 无（仅完整压缩） | 工具结果裁剪 | 无 | 工具结果裁剪 + 清除 |
| **Post-compact 恢复** | 文件重读 + Plan + Skill + Agent 附件 + Deferred tools | Initial context 注入 + Ghost snapshots | 自动 Continue 消息 | 无 | **无** |
| **多模态处理** | Strip image/document 为占位符 | Strip image | 无 | Strip ThinkPart | **无** |
| **Prompt Cache** | Cache-sharing fork + cache_edit 微压缩 | Mid-turn cache 前缀保留 | 无 | 无 | **无** |
| **摘要质量** | `<analysis>` + `<summary>` 双 XML 块 + 格式化 | Summary prefix + 用户消息保留 | 结构化模板 | XML 结构化输出 | `<analysis>` + `<summary>` 双块 |
| **PTL 重试** | 按分组截断头部 + 最多 3 次 | 从头部逐个移除 + backoff | 无 | 无 | 丢弃最老 turn + 最多 3 次 |
| **熔断机制** | 连续失败 3 次停试 | 最大重试 + backoff | 无 | 无 | **无** |
| **Hook 系统** | Pre-compact + Post-compact + Session-start hooks | 无 | Plugin 系统注入 | 无 | **无** |
| **摘要格式化** | `formatCompactSummary` 去标签 | Summary prefix 拼接 | LLM 原文 | 无特殊处理 | `<summary>` 提取 |
| **远程压缩** | 无（全本地） | 支持 OpenAI remote compact API | 无 | 无 | 无 |

---

## 2. 升级路线（按优先级排序）

### Phase 1: 基础加固（1-2 周）

> 目标：补齐 Astrcode 缺失的基础能力，降低压缩失败率和信息丢失。

#### 2.1.1 Post-compact 附件恢复

**为什么**：当前压缩后所有已读文件、Plan 状态全部丢失，Agent 需要重新读取文件才能继续工作。Claude Code 在压缩后会：
- 重新读取最近访问的 5 个文件（50K token 预算）
- 恢复 Plan 文件附件
- 恢复已调用的 Skill 内容
- 重新注入 Deferred tools schema
- 重新注入 MCP 指令

**实现方案**：
1. 在 `CompactResult` 中增加 `post_compact_attachments: Vec<PostCompactAttachment>` 字段
2. 在 `compaction_runtime.rs` 的 `CompactionRebuilder` trait 中增加 `build_post_compact_attachments()` 方法
3. 实现文件重读附件：跟踪 `readFileState`，压缩后选取最近 N 个文件（参考 Claude Code 的 `createPostCompactFileAttachments`）
4. 压缩后触发 `ContextPipeline` 的重新注入逻辑（system prompt attachments、MCP 指令等）

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/compaction.rs`
- `crates/runtime-agent-loop/src/compaction_runtime.rs`
- `crates/core/src/projection/agent_state.rs`

#### 2.1.2 多模态消息处理

**为什么**：当前压缩时会直接将 image/document 消息发给 LLM 生成摘要，可能因 prompt 过长而失败。Claude Code 和 Codex 都在压缩前将多模态内容替换为占位符。

**实现方案**：
1. 在 `compact_input_messages()` 中增加过滤：将 `Image`/`Document` 类型的消息内容替换为 `[image]`/`[document]` 占位文本
2. 保留文本内容不变
3. 与 `LlmMessage` 的多模态扩展同步设计（当前 `LlmMessage::User.content` 是 `String`，后续扩展为 `Vec<ContentPart>` 时适配）

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/compaction.rs`
- `crates/core/src/action.rs`（`UserMessageOrigin` 和 `LlmMessage` 类型扩展）

#### 2.1.3 熔断机制

**为什么**：Claude Code 观测到 1,279 个会话出现 50+ 次连续压缩失败，每天浪费 ~250K API 调用。Astrcode 当前无熔断保护。

**实现方案**：
1. 在 `CompactionRuntime` 中增加 `consecutive_failures: AtomicUsize` 字段
2. 压缩成功时 reset，失败时 +1
3. 当 `consecutive_failures >= 3` 时跳过自动压缩
4. 手动压缩不受熔断限制

**涉及文件**：
- `crates/runtime-agent-loop/src/compaction_runtime.rs`

---

### Phase 2: Prompt Cache 优化（2-3 周）

> 目标：利用 Prompt Cache 减少压缩后的 API 成本和延迟。

#### 2.2.1 Cache-sharing Fork 压缩路径

**为什么**：Claude Code 使用 fork agent 复用主对话的 cached prefix 来执行压缩请求。这意味着压缩 API 调用本身可以命中 prompt cache（system prompt + tools + 前缀消息），大幅降低成本。

**实现方案**：
1. 在 `LlmProvider` trait 中增加 `fork_for_compaction()` 方法，允许复用已有连接/缓存
2. 在 `auto_compact()` 中尝试走 cache-sharing 路径：发送相同的 system prompt 和工具定义
3. 如果 cache sharing 失败，回退到当前的独立请求路径

**涉及文件**：
- `crates/runtime-llm/src/lib.rs`（`LlmProvider` trait）
- `crates/runtime-agent-loop/src/context_window/compaction.rs`

#### 2.2.2 时间触发微压缩

**为什么**：Claude Code 发现当距上次 assistant 消息的时间间隔超过阈值（通常 5-15 分钟），server 端 prompt cache 已过期，此时清除旧工具结果不会浪费缓存。Astrcode 的微压缩仅按 token 压力触发。

**实现方案**：
1. 在 `microcompact.rs` 中增加时间触发逻辑：检查最后 assistant 消息的时间戳
2. 当间隔 > 阈值时，清除所有非最近 N 个 compactable 工具的结果
3. 通过配置暴露时间阈值和保留数量

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/microcompact.rs`
- `crates/runtime-config/`（配置暴露）

---

### Phase 3: Partial Compact（2-3 周）

> 目标：支持用户选择压缩方向，减少不必要的全量摘要。

#### 2.3.1 Partial Compact (`from` / `up_to` 方向)

**为什么**：Claude Code 支持 `partialCompactConversation`，允许用户指定从某个消息开始压缩：
- `from`：保留前缀（cache 友好），压缩后缀
- `up_to`：保留后缀（当前行为），压缩前缀

Codex 也有类似的 `InitialContextInjection::BeforeLastUserMessage` 模式。

**实现方案**：
1. 扩展 `CompactConfig` 增加 `direction: CompactDirection { Suffix, From(usize), UpTo(usize) }`
2. 修改 `split_for_compaction()` 支持按方向分割
3. 在 `compaction_runtime.rs` 增加 `PartialCompactionStrategy`
4. 前端增加消息选择器 UI，允许用户选择压缩范围
5. `from` 方向需要处理 prompt cache 兼容性

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/compaction.rs`
- `crates/runtime-agent-loop/src/compaction_runtime.rs`
- `crates/core/src/action.rs`
- 前端组件

#### 2.3.2 Incremental Recompact

**为什么**：当前每次压缩都重新摘要完整前缀。如果之前已有摘要，应该只增量压缩新增内容，然后将新旧摘要合并。Codex 在 `compact.rs` 中保留 `CompactSummary` 消息用于增量折叠。

**实现方案**：
1. 修改 `auto_compact()` 入口：检测前缀中是否已有 `CompactSummary` 消息
2. 如果有，只压缩上次摘要之后的新消息
3. 将新摘要与旧摘要合并（可选：再调用一次 LLM 合并两个摘要）
4. 避免每次重新摘要整个历史

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/compaction.rs`
- `crates/runtime-agent-loop/src/compaction_runtime.rs`

---

### Phase 4: 智能化（3-4 周）

> 目标：通过 Hook 系统和更精细的 prompt 工程提升摘要质量。

#### 2.4.1 Pre/Post Compact Hook 系统

**为什么**：Claude Code 有完整的 hook 生命周期：
- `PreCompact` hook：允许插件修改压缩 prompt 或注入自定义指令
- `PostCompact` hook：允许插件在压缩后执行恢复操作
- `SessionStart` hook：压缩后重新触发

OpenCode 也有 `Plugin.trigger("experimental.session.compacting")` 支持。

**实现方案**：
1. 在 `core` 中定义 `CompactHookEvent` 和 `CompactHookResult` 类型
2. 在 `runtime` 中实现 hook 执行：在 `auto_compact()` 前后调用
3. 允许插件注入自定义压缩指令（类似 Claude Code 的 `customInstructions`）
4. 压缩后触发 SessionStart hook 以恢复 session 级别的上下文注入

**涉及文件**：
- `crates/core/src/`（hook 事件类型）
- `crates/runtime-agent-loop/src/compaction_runtime.rs`
- `crates/plugin/`（hook 注册）

#### 2.4.2 摘要 Prompt 工程升级

**为什么**：当前 Astrcode 的 `build_compact_system_prompt` 已经有 9 段结构，但缺少：
- Claude Code 的 `<analysis>` scratchpad 机制（让模型先草拟再输出，提升质量）
- Claude Code 的 NO_TOOLS 强制约束（防止压缩时调用工具）
- Kimi CLI 的压缩优先级排序
- OpenCode 的 plugin 可定制 prompt

**实现方案**：
1. 增强 system prompt：加入 `<analysis>` 草稿区块要求
2. 加入 NO_TOOLS 约束（在 system prompt 开头明确禁止工具调用）
3. 在 `extract_summary()` 中先提取 `<analysis>` 再提取 `<summary>`，最终只保留 summary
4. 参照 Claude Code 的 `formatCompactSummary()` 做标签清理
5. 支持通过 Hook 注入自定义指令

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/compaction.rs`（`build_compact_system_prompt`、`extract_summary`）

#### 2.4.3 精确 Token 计数

**为什么**：当前使用 4 chars/token 启发式估算，误差可达 ±30%。Codex 使用 `approx_token_count`，Claude Code 使用 `roughTokenCountEstimation` 并有 Provider 报告的精确 usage 数据作为锚定。Astrcode 的 `TokenUsageTracker` 已经优先使用 Provider 报告的 usage，但 context 估算仍是启发式。

**实现方案**：
1. 短期：在 `TokenUsageTracker` 中利用 Provider 返回的 `usage.input_tokens` 作为实际 token 数锚点
2. 中期：为各 Provider 实现 tokenizer（tiktoken for OpenAI-compatible）
3. 长期：将 `estimate_message_tokens` 替换为 Provider-native 精确计数

**涉及文件**：
- `crates/runtime-agent-loop/src/context_window/token_usage.rs`
- `crates/runtime-llm/src/lib.rs`

---

## 3. 实施优先级总览

| Phase | 核心改进 | 预估工作量 | 影响范围 |
|-------|----------|-----------|----------|
| **Phase 1** | Post-compact 恢复 + 多模态 + 熔断 | 1-2 周 | 压缩可靠性 ⬆ |
| **Phase 2** | Cache sharing + 时间微压缩 | 2-3 周 | API 成本 ⬇ |
| **Phase 3** | Partial compact + 增量重压缩 | 2-3 周 | 灵活性 ⬆ |
| **Phase 4** | Hook 系统 + Prompt 工程 + 精确计数 | 3-4 周 | 摘要质量 ⬆ |

建议按 Phase 1 → Phase 4 顺序实施，每个 Phase 完成后都有独立的可观测收益。

---

## 4. 关键设计决策参考

### 4.1 从 Claude Code 学到的

1. **Cache-sharing fork**：压缩请求复用主对话的 cached prefix，节约 30-50% 的 token 成本
2. **时间触发微压缩**：利用 cache TTL 判断是否需要清除旧工具结果，避免浪费 cache
3. **Partial compact**：`from` 方向保持前缀 cache，`up_to` 方向保持后缀上下文
4. **Post-compact 恢复**：必须恢复文件内容、Plan、Skill、Deferred tools，否则 Agent 失忆
5. **熔断器**：3 次连续失败后停止自动压缩，手动压缩不受限制
6. **`<analysis>` scratchpad**：让模型先草拟分析再输出摘要，显著提升摘要质量

### 4.2 从 Codex 学到的

1. **Mid-turn 压缩**：在 turn 执行过程中遇到 context 超限时实时压缩（`InitialContextInjection::BeforeLastUserMessage`）
2. **Remote compact API**：支持调用 Provider 端的压缩 API（如 OpenAI 的），减少客户端 token 消耗
3. **用户消息保留**：压缩后保留所有真实用户消息（在 token 预算内），保持用户意图可见
4. **Ghost snapshots**：保留 undo 历史的快照，压缩后 `/undo` 仍然可用

### 4.3 从 OpenCode 学到的

1. **Plugin 注入**：允许第三方插件在压缩时注入上下文或替换 prompt
2. **Prune 策略**：从后往前扫描，保护最近 N 个 turn 的工具结果，清除超出预算的旧结果
3. **Auto-continue**：压缩后自动生成一条 "Continue" 消息，让 Agent 无缝继续工作

### 4.4 从 Kimi CLI 学到的

1. **压缩优先级排序**：当前任务 > 错误/修复 > 代码演进 > 系统上下文 > 设计决策 > TODO
2. **Protocol-based 策略**：`Compaction` trait 允许灵活替换压缩算法
3. **简洁的 `max_preserved_messages` 参数**：以消息数而非 token 数控制保留范围

---

## 5. 风险和注意事项

1. **向后兼容**：`CompactTrigger`、`StorageEvent::CompactApplied` 等类型变更需同步到 JSONL 存储层
2. **Provider 差异**：不同 LLM Provider 的 cache 机制不同，cache-sharing 需要按 Provider 适配
3. **多模态扩展**：`LlmMessage::User.content` 从 `String` 扩展为 `Vec<ContentPart>` 是大范围变更，需协调
4. **增量重压缩的摘要质量**：合并两次摘要可能丢失中间细节，需实测决定是 LLM 合并还是简单拼接
5. **Partial compact 的 cache 一致性**：`from` 方向保持 cache 前缀，但 `up_to` 方向会破坏 cache，需在 UI 中提示
