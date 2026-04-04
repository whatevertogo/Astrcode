# Compact 系统优化计划

> 基于五个主流 coding agent (Claude Code, Codex, OpenCode, Kimi CLI, pi-mono) 的对比分析
> 结合 Astrcode 现有架构进行优先级排序

## 一、对比分析结论

### 各方案核心优势

| 项目 | 核心优势 | 值得借鉴的点 |
|------|---------|-------------|
| **Claude Code** | 工业级稳定性 | 多层缓冲区设计(13K/20K/20K/3K)、电路熔断、Post-compact 附件恢复、Cache-sharing fork、时间微压缩 |
| **Codex** | 灵活的压缩方向 | Mid-turn 压缩、InitialContextInjection 策略(前缀/后缀注入)、OpenAI remote compact API |
| **OpenCode** | 分层渐进压缩 | 三层架构(prune→process→create)、专用 compact agent(无工具权限)、Auto-continue |
| **pi-mono** | 增量摘要 | Update prompt 模式合并旧摘要、Extension hook 拦截、Branch 摘要导航、文件操作跟踪 |
| **Kimi CLI** | 简洁有效 | 50K reserved buffer 简单可靠、压缩内容优先级排序(当前任务>错误>代码>上下文)、JSONL 检查点 |
| **Astrcode** | 架构清晰 | Policy/Strategy/Rebuilder 三层分离、ContextPipeline stage 管线、413 reactive compact |

### Astrcode 缺失的关键能力

| 缺失项 | 影响 | 参考项目 | 优先级 |
|--------|------|---------|--------|
| Post-compact 附件恢复 | 压缩后"失忆"，Agent需要重新探索文件 | Claude Code, Codex | 🔴 P0 |
| 电路熔断 | 连续失败时浪费API调用 | Claude Code | 🔴 P0 |
| 多模态消息处理 | 压缩时将image/document发给LLM导致失败 | Claude Code, Codex | 🟡 P1 |
| Prune 机制 | 无法清理已完成turn的旧工具结果 | OpenCode | 🟡 P1 |
| Cache-sharing fork | 压缩时无法复用主对话cache | Claude Code | 🟡 P1 |
| Auto-continue nudge | 压缩后Agent不知道继续做什么 | OpenCode | 🟡 P1 |
| 增量重压缩 | 每次都重新摘要完整历史 | pi-mono, Codex | 🟢 P2 |
| Partial Compact方向 | 只支持后缀保留 | Claude Code, Codex | 🟢 P2 |
| Hook 系统 | 插件无法介入压缩流程 | Claude Code, pi-mono | 🟢 P2 |
| 文件操作跟踪 | 摘要中无结构化文件信息 | pi-mono | 🟢 P2 |
| Prompt 工程升级 | 摘要质量可提升 | Claude Code, Kimi CLI | 🟢 P2 |
| 时间微压缩 | 可以在cache过期时清理 | Claude Code | 🔵 P3 |
| Mid-turn 压缩 | 无法在turn中间处理超限 | Codex | 🔵 P3 |
| Context Usage 可视化 | 用户无法看到上下文使用 | Claude Code, OpenCode | 🔵 P3 |

---

## 二、优化阶段规划

### Phase 1：基础加固（防止数据丢失和API浪费）

> **目标**：补齐基础能力，确保压缩不浪费API、不失忆

#### 1.1 Post-compact 附件恢复

**问题**：压缩后已读文件内容、Plan状态全部丢失，Agent需要重新探索，浪费token。

**Claude Code参考**：`createPostCompactFileAttachments()` 重新读取最近5个文件(50K token预算)、恢复Plan文件和Skill内容。

**Codex参考**：`InitialContextInjection::BeforeLastUserMessage` 将初始上下文注入到最后用户消息前。

**设计方案**：
```
FileAccessTracker（新增结构体，runtime-agent-loop内部）
  - 从 StorageEvent::ToolResult 中提取 readFile/editFile/writeFile 的路径
  - 按时间倒序维护访问记录
  
PostCompactAttachment（枚举）
  - FileContent(path, content)
  - PlanState(plan)
  - SkillContent(name, content)
  - McpInstructions

CompactionRebuilder trait扩展
  - build_post_compact_attachments() → Vec<PostCompactAttachment>
  
Token预算：总附件50K tokens，单文件5K max，最多5个文件
```

#### 1.2 电路熔断

**问题**：Claude Code观测到1279个会话出现50+次连续压缩失败，每天浪费250K API调用。

**设计方案**：
```rust
ThresholdCompactionPolicy（扩展）
  - consecutive_failures: AtomicUsize
  - MAX_CONSECUTIVE_FAILURES = 3
  
should_compact()逻辑：
  - consecutive_failures >= 3 → 返回None（仅Auto/Reactive）
  - Manual不经过Policy，直接调用Strategy绕过熔断
  - record_success()重置为0，record_failure()+1
```

#### 1.3 多模态消息处理

**问题**：压缩时将image/document消息直接发给LLM生成摘要，可能因prompt过长而失败。

**Claude Code参考**：`stripImagesFromMessages()` 将Image/Document替换为占位符。

**设计方案**：
```rust
compact_input_messages()增加过滤：
  - 检测消息中的多模态内容
  - 将Image/Document替换为[image] [document]占位文本
  - 保留文本内容不变
```

#### 1.4 精确 Token 计数

**问题**：当前4 chars/token启发式估算误差可达±30%。

**设计方案**：
```rust
TokenUsageTracker（扩展现有anchored_budget_tokens机制）
  - 记录usage.input_tokens作为单次请求锚点
  - estimate_request_tokens_anchored():
    - 有锚点时: anchor.tokens + 增量消息估算
    - 无锚点时: 回退到现有4 chars/token全量估算
  - 中期: 各Provider实现tokenizer
  - 长期: Provider-native精确计数
```

### Phase 2：Prompt Cache 优化（降低成本和延迟）

> **目标**：利用Prompt Cache减少压缩后的API成本和等待时间

#### 2.1 Cache-sharing Fork 压缩

**问题**：压缩时发送独立请求，无法复用主对话cached prefix，浪费token。

**Claude Code参考**：fork agent复用主对话cached prefix，节约30-50% token，98%缓存命中率。

**设计方案**：
```rust
LlmProvider trait扩展
  - supports_cache_sharing() -> bool (默认false)
  - fork_for_compaction() -> Option<Self> (复用缓存的子请求)

Anthropic实现：
  - 复用HTTP连接 + cache_control marker
  - 压缩请求发送相同的system prompt（带cache_control）+ 前缀消息
  
OpenAI v1：
  - is_cache_sharing_supported = false
  - 走独立请求路径
```

#### 2.2 Auto-continue Nudge

**问题**：压缩后Agent不知道该继续做什么。

**OpenCode参考**：自动压缩成功后插入"Continue"消息，Agent无缝继续。

**设计方案**：
```rust
auto_compact()成功后：
  - 若触发原因为Auto
  - 在compacted_messages()中追加AutoContinueNudge消息
  - "The conversation was compacted. Continue from where you left off."
```

#### 2.3 Prune 机制

**问题**：微压缩仅在当前消息操作，不会清理已完成turn的旧工具结果。

**OpenCode参考**：prune()从后往前扫描，保护最近40K tokens工具输出，超出部分替换占位。

**设计方案**：
```rust
PostTurnPruneStage（context pipeline新增）
  prune_old_tool_results():
    - 从后往前累加token
    - 超出保护区的旧工具结果替换为占位文本
    - 保护最近N tokens不被prune
    - 最小prune阈值：低于不执行
    
保护规则：
  - 最近的2个turn不受影响
  - 特定工具(如skill)结果不prune
```

#### 2.4 时间触发微压缩

**问题**：当server端prompt cache已过期时清除旧工具结果不会浪费缓存。

**Claude Code参考**：超过30分钟清除旧工具结果，因为cache已过期（1h TTL）。

**设计方案**：
```rust
MicrocompactTrigger枚举
  - TokenPressure（现有）
  - CacheExpired（新增）
  - Both

检查逻辑：
  - dist(last_assistant_timestamp, now) > threshold(默认15分钟)
  - 清除所有非最近N个compactable工具的结果
```

### Phase 3：Partial & Incremental Compact（灵活压缩）

> **目标**：支持灵活压缩方向，增量重压缩减少浪费

#### 3.1 Partial Compact 方向

**问题**：只支持后缀保留压缩。

**Claude Code参考**：partialCompact支持from(保留前缀，压缩后缀，cache友好)和up_to(保留后缀，压缩前缀)。

**设计方案**：
```rust
CompactConfig扩展
  - direction: CompactDirection
    - Suffix（现有默认）
    - From(usize)（保留前缀）
    - UpTo(usize)（保留后缀，现有行为）
    
split_for_compaction()支持按方向分割
PartialCompactionStrategy（新增trait）
```

#### 3.2 增量重压缩

**问题**：每次都重新摘要完整历史，浪费token且丢失信息。

**pi-mono参考**：如有已有摘要，使用"update" prompt将新信息合并到现有摘要。

**Codex参考**：保留CompactSummary消息，检测到SUMMARY_PREFIX标记。

**设计方案**：
```rust
incremental_recompact()：
  - 检测前缀中是否已有CompactSummary消息
  - 有：只压缩上次摘要之后的新消息
  - 生成增量摘要后与旧摘要合并
  - LLM合并：提供旧摘要+新内容要求合并（pi-mono的update prompt模式）
  - 简单合并：拼接（fallback）
```

#### 3.3 文件操作跟踪

**问题**：摘要中缺乏对读写文件的结构化跟踪。

**pi-mono参考**：extractFileOperations()累积跟踪read/written/edited文件，摘要追加<read-files>和<modified-files> XML段。

**设计方案**：
```rust
FileOperationSet{read, written, edited}
  - 提取工具调用中的文件路径参数
  - 累积维护跨多次压缩
  - 在摘要prompt中注入文件操作信息
  - 在摘要输出中增加文件列表段
```

### Phase 4：智能化（Hook系统和Prompt工程）

> **目标**：提升摘要质量和可扩展性

#### 4.1 Pre/Post Compact Hook 系统

**问题**：插件无法介入压缩流程。

**Claude Code参考**：PreCompact/PostCompact/SessionStart三类hook。
**pi-mono参考**：session_before_compact event（可取消或提供自定义摘要）。

**设计方案**：
```rust
CompactHookEvent/CompactHookResult（core中定义）
Hook类型：
  - PreCompact（可修改prompt，允许取消）
  - PostCompact（可执行恢复）
  - SessionRestore（压缩后恢复session上下文）

复用crates/plugin/的JSON-RPC通信通道
参考现有PolicyHook的注册和链式执行机制
```

#### 4.2 摘要 Prompt 工程升级

**Claude Code参考**：analysis scratchpad + NO_TOOLS约束 + formatCompactSummary()标签清理。
**Kimi CLI参考**：压缩优先级排序（当前任务>错误修复>代码演进>系统上下文>设计决策>TODO）。

**设计方案**：
```rust
extract_summary()改进：
  - 增加analysis块存在性校验
  - 缺失时记录warning但不阻断

build_compact_system_prompt()改进：
  - NO_TOOLS约束移到最前面
  - 更强硬措辞
  - 参考Kimi CLI的内容优先级

format_compact_summary()新增：
  - 标签清理（去除残留XML标签）
  - 规范化空白
  
支持通过Hook注入自定义指令
```

#### 4.3 Context Usage 可视化

**Claude Code参考**：ContextVisualization组件展示token按类别分布、/context命令。

**设计方案**：
```
/context命令：显示上下文使用量、按类别分布
前端context usage指示器
压缩前显示token节省预估
```

#### 4.4 Mid-turn 压缩

**Codex参考**：Mid-turn压缩在model response完成后检测token超限，inline触发压缩。

**设计方案**：
```
Mid-turn压缩：
  - 在turn执行过程中检测token超限
  - inline触发压缩
  - 压缩完成后恢复该turn
```

---

## 三、实施优先级建议

| 阶段 | 事项 | 优先级 | 预计难度 | 预期收益 |
|------|------|--------|---------|---------|
| **P1-1** | Post-compact 附件恢复 | 🔴 | 中 | 高（解决"失忆"问题） |
| **P1-2** | 电路熔断 | 🔴 | 低 | 中（防止API浪费） |
| **P1-3** | 多模态消息处理 | 🟡 | 低 | 高（防止压缩失败） |
| **P1-4** | 精确 Token 计数 | 🟡 | 中 | 高（更准确阈值） |
| **P2-1** | Cache-sharing fork | 🟡 | 中高 | 高（节省30-50% token） |
| **P2-2** | Auto-continue | 🟡 | 低 | 中（用户体验） |
| **P2-3** | Prune 机制 | 🟢 | 中 | 中（上下文清理） |
| **P2-4** | 时间微压缩 | 🔵 | 低 | 中（cache感知清理） |
| **P3-1** | Partial 方向 | 🟢 | 中 | 中（灵活选择） |
| **P3-2** | 增量重压缩 | 🟢 | 高 | 高（节省token） |
| **P3-3** | 文件跟踪 | 🟢 | 低 | 中（摘要结构化） |
| **P4-1** | Hook 系统 | 🟢 | 高 | 高（可扩展性） |
| **P4-2** | Prompt 工程 | 🟢 | 低 | 中（摘要质量） |
| **P4-3** | Context 可视化 | 🔵 | 中 | 低（用户体验） |
| **P4-4** | Mid-turn 压缩 | 🔵 | 高 | 低（边界场景） |

---

## 四、关键技术决策

1. **Post-compact 附件恢复放在 Phase 1**：直接解决"压缩后失忆"的核心痛点，是用户体验的关键
2. **电路熔断放在 Policy 层**：符合现有"Policy做决策、Strategy做执行"架构
3. **Cache-sharing fork放在 Phase 2**：需要 LlmProvider trait 扩展，依赖 Provider 适配
4. **增量重压缩放在 Phase 3**：需要理解现有Summary消息的处理链路，复杂度较高
5. **Hook系统放在 Phase 4**：需要对插件系统深度集成，工作量最大

---

## 五、成功指标

- [ ] 压缩失败率 < 1%（当前未知，需建立基线）
- [ ] 压缩后 Agent 重新访问文件的次数减少 > 50%
- [ ] 压缩节省的 token 占总对话 token 的 > 30%
- [ ] 压缩引起的 API 调用浪费 < 0.1%
- [ ] 压缩后对话可以继续正常的成功率 > 99%
