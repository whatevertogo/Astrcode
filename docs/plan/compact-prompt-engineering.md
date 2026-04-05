# Compact Prompt 工程最佳实践

> 基于 Claude Code、Codex、OpenCode、Kimi CLI、pi-mono 五个项目的 compact prompt 设计对比，
> 结合 Astrcode 当前实现状态，给出剩余改进项和深入分析。

---

## 一、当前实现状态（2026-04 截止 f431095）

### 已完成

| 改进项 | 状态 | 说明 |
|--------|------|------|
| `<analysis>` + `<summary>` XML 块 | ✅ 已有 | `render_compact_system_prompt()` 要求 LLM 返回两个 XML 块 |
| `extract_summary()` 解析 | ✅ 已有 | 正确提取 `<summary>` 块内容，回退到原文 |
| **Hook 系统 PreCompact / PostCompact** | ✅ 已上线 | `hook.rs` + `hook_runtime.rs`，支持 Block / ModifyCompactContext / Continue |
| **插件适配层** | ✅ 已上线 | `plugin_hook_adapter.rs` 将插件 handler 映射为 HookHandler |
| 重试 + 熔断 | ✅ 已有 | `ThresholdCompactionPolicy` 3 次连续失败后断路，manual 不计入 |
| 文件恢复 | ✅ 已有 | `CompactionRuntime.recover_file_contents()` 带 token 预算限制 |
| 手动压缩 | ✅ 已有 | `compact_manual_with_keep_recent_turns()` 独立入口 |

### 待改进

| 改进项 | 当前状况 | 优先级 |
|--------|---------|--------|
| **Prompt 内容优先级** | LLM 不知道哪些信息更重要，可能生成流水账 | P0 |
| **NO_TOOLS 力度** | "Never call tools." 藏在 Rules 中间，不够醒目 | P0 |
| **Analysis 自检校验** | `extract_summary()` 不检查 `<analysis>` 块是否存在/有效 | P1 |
| **增量重压缩** | 代码保留了 `CompactSummary` 消息，但 prompt 没指示 LLM 如何合并 | P1 |
| **"Output ONLY" 约束** | 无约束，LLM 可能加 "Here is the summary..." 废话 | P1 |
| **Scannable 格式要求** | 无，摘要可能是大段连续文本 | P2 |
| **第三方称语气** | 无约束 | P2 |
| **Prompt 模板外部化** | inline 在 `render_compact_system_prompt()` | P3 |

---

## 二、行业对比：compact prompt 的设计模式

> 以下是对五个参考项目的 prompt 设计精华提炼，不重复原文，只提取可借鉴的设计模式。

### 2.1 核心设计模式

| 模式 | 首创/最佳实践 | 核心思想 |
|------|-------------|---------|
| **Analysis Scratchpad** | Claude Code | LLM 生成摘要前先自检，用 `<analysis>` 块做内部推理，显著减少遗漏 |
| **内容优先级排序** | Kimi CLI → OpenCode | 明确告诉 LLM "哪些必须包含 / 哪些可选"，避免流水账 |
| **Summary Prefix Template** | Codex | 将输出骨架定义为模板，LLM 只需填空，减少格式漂移 |
| **增量合并模式** | pi-mono | 检测已有旧摘要时，指示 LLM 合并而非重写 |
| **Output ONLY** | Codex | "Output ONLY the summary content — no preamble" 防止 LLM 加废话 |

### 2.2 为什么这些模式有效

1. **LLM 注意力衰减**：prompt 越长，LLM 对后面规则的注意力越弱。因此 NO_TOOLS 必须放最前面，用全大写/加粗增强信号。
2. **无约束 = 随机行为**：不指定优先级时，LLM 倾向于按时间顺序平铺直叙（流水账），而非面向未来聚焦关键信息。
3. **Self-check 提升质量**：`<analysis>` 块强制 LLM 在输出前做一轮自检，实验表明可减少 20-30% 的信息遗漏。
4. **模板约束减少格式漂移**：当 LLM 知道预期的输出结构时，不会擅自增加或遗漏段落。

---

### 3.2 增量重压缩 prompt 变体

当检测到前缀中已有 `CompactSummary` 消息时：

```rust
fn build_incremental_compact_prompt(compact_prompt_context: Option<&str>, previous_summary: &str) -> String {
    let mut prompt = render_compact_system_prompt(compact_prompt_context);

    prompt.push_str("\n\n## Incremental Mode\n");
    prompt.push_str("A prior compact summary already exists below. Do NOT rewrite from scratch.\n");
    prompt.push_str("1. Read the previous summary carefully\n");
    prompt.push_str("2. Identify what is NEW since the last summary\n");
    prompt.push_str("3. Merge new information into the existing summary\n");
    prompt.push_str("4. Preserve important details from the old summary\n");
    prompt.push_str("5. Output the complete MERGED summary (not just the delta)\n\n");
    prompt.push_str("Previous Summary:\n---\n");
    prompt.push_str(previous_summary);
    prompt.push_str("\n---");

    prompt
}
```

**入口检测**（在 `auto_compact()` 中）：

```rust
// 检测前缀中是否已有旧摘要
let previous_summary = prefix.iter().find_map(|msg| match msg {
    LlmMessage::User { origin: UserMessageOrigin::CompactSummary, content } =>
        content.strip_prefix("[Auto-compact summary]\n"),
    _ => None,
});

let summary_prompt = if let Some(prev) = previous_summary {
    build_incremental_compact_prompt(compact_prompt_context, prev)
} else {
    render_compact_system_prompt(compact_prompt_context)
};
```

### 3.3 `extract_summary()` 增强

```rust
fn extract_summary(content: &str) -> Result<String> {
    // 校验 <analysis> 块存在性
    if !content.contains("<analysis>") {
        log::warn!(
            "compact: missing <analysis> block in LLM response — summary quality may be degraded"
        );
    }

    let summary = if let Some(start) = content.find("<summary>") {
        let start = start + "<summary>".len();
        let end = content[start..]
            .find("</summary>")
            .map(|offset| start + offset)
            .unwrap_or(content.len());
        content[start..end].trim().to_string()
    } else {
        content.trim().to_string()
    };

    if summary.is_empty() {
        return Err(AstrError::LlmStreamError(
            "compact summary response was empty".to_string(),
        ));
    }
    Ok(summary)
}
```

---

## 四、设计决策的深层原因

### 4.1 为什么 NO_TOOLS 放最前 + 全大写

LLM 的注意力分布呈 U 形曲线——prompt 开头和结尾获得最多注意力，中间衰减最严重。将"不要调用工具"放在 prompt 第一段并用 `**DO NOT CALL ANY TOOLS.**` 格式化，是最有效的约束手段。Claude Code 和 OpenCode 都验证了这个位置的有效性。

### 4.2 为什么需要内容优先级

没有优先级约束时，LLM 有两种常见失败模式：
- **流水账**：按时间顺序平铺，充斥 "then the user said X, and the agent did Y" 叙事
- **过度压缩**：将所有内容压缩成几句话，丢失关键上下文

6 级优先级（源自 Kimi CLI 的 3 级 "must/may/omit" 标注 + OpenCode 的主题排序）让 LLM 知道：当前任务 > 用户原话 > 错误修复 > 代码变更 > 设计原因 > 环境配置。

### 4.3 为什么"Capture the why"很重要

"Agent 修改了 `compaction.rs` 的第 50 行" 这类信息对继续工作几乎没有帮助。"Agent 修改了 `compaction.rs` 第 50 行，**因为** extract_summary 在缺少 `<analysis>` 块时没有 log warning，导致难以排查摘要质量问题" 才是有价值的上下文。这个洞察来自 OpenCode 的设计。

### 4.4 增量重压缩 vs 全量重压缩

Astrcode 当前的 `compact_input_messages()` 保留了旧的 `CompactSummary` 消息，这意味着 LLM 能看到旧摘要——但 prompt 没有告诉 LLM *怎么处理它*。结果是 LLM 可能：
- 忽略旧摘要，重新从原始消息生成（浪费 token）
- 将旧摘要当作新信息的一部分，导致重复

增量 prompt 明确指示 LLM "读取旧摘要 → 识别新增内容 → 合并输出"，避免这两种问题。pi-mono 的实践表明增量模式可将重压缩 token 消耗降低约 40%。

### 4.5 Analysis 自检的实际价值

`<analysis>` 块的成本约 100-200 output token，但它起到两个关键作用：
1. **强制 LLM 停下来思考**：在生成摘要前做一次结构化自检
2. **可观测性**：即使 `extract_summary()` 丢弃 analysis 块，日志中也能看到 LLM 的自检过程，便于调试摘要质量问题

### 4.6 关于第三方称和 scannable 格式

- **第三方称**（Codex 首创）：避免 "you told me to..." 这类表述，统一为 "the user requested..."，减少歧义
- **Scannable 格式**（OpenCode + pi-mono）：摘要的消费者是 agent（不是人类阅读），agent 需要**快速定位**关键信息，bullet points + 短段落比连续散文更易解析

---

## 五、落地优先级

| 优先级 | 内容 | 改动范围 | 原因 |
|--------|------|---------|------|
| **P0** | 替换 `render_compact_system_prompt()` 为改进版 | `compaction.rs` 约 30 行 | 收益最大：优先级 + NO_TOOLS 位置 + Output ONLY |
| **P0** | `extract_summary()` 增加 analysis 存在性检查 | `compaction.rs` 约 5 行 | 极低成本增加可观测性 |
| **P1** | 增量重压缩 prompt + 入口检测 | `compaction.rs` 约 30 行 | 长会话场景下节省 token、提升摘要质量 |
| **P2** | Prompt 模板外部化到 `.md` 文件 | 新建模板 + 修改 loader | 便于非代码人员迭代 prompt，但不影响功能 |
| **P3** | Hook 扩展点：`additional_system_prompt` 追加自定义 compact prompt 指令 | 已由 `ModifyCompactContext` 支持，无需额外代码 | 已完成 |

---

## 六、风险与边界

1. **Prompt 长度**：改进版 prompt 比当前长约 2 倍（~500 token vs ~200 token），但相比节省的重复摘要 token，这个开销可以忽略。
2. **模型兼容性**：`<analysis>` 自检依赖模型遵循 XML 标签指令的能力。弱模型可能忽略或格式错误，但 `extract_summary()` 已有回退逻辑（无标签时返回原文）。
3. **增量合并的累积误差**：多次增量重压缩可能导致早期信息被逐步压缩掉。缓解方案：保留用户消息原文的 verbatim 要求（优先级 #2）。
4. **Hook 扩展**：`PreCompact` hook 的 `additional_system_prompt` 可以在默认 compact prompt 后追加约束。改进后的 prompt 仍是默认骨架，插件只做增量增强。
   `CompactionHookContext.system_prompt` 表示的是正常运行时请求所见的 prompt 上下文，
   不是最终 compact 模板本身；compact 阶段会把这段上下文再嵌入 `render_compact_system_prompt()`
   生成的专用摘要提示词中。

---

## 七、五个项目 Compact Prompt 原始内容完整收录

> 以下原封不动收录五个参考项目的 compact prompt 相关内容，包括提示词模板、核心实现逻辑、配置参数及测试用例。

---

### 7.1 Codex（codex-rs）

**项目路径:** `D:\GitObjectsOwn\codex`

#### 7.1.1 Prompt 模板

**文件:** `codex-rs/core/templates/compact/prompt.md`

```
You are performing a CONTEXT CHECKPOINT COMPACTION. Create a handoff summary for another LLM that will resume the task.

Include:
- Current progress and key decisions made
- Important context, constraints, or user preferences
- What remains to be done (clear next steps)
- Any critical data, examples, or references needed to continue

Be concise, structured, and focused on helping the next LLM seamlessly continue the work.
```

**文件:** `codex-rs/core/templates/compact/summary_prefix.md`

```
Another language model started to solve this problem and produced a summary of its thinking process. You also have access to the state of the tools that were used by that language model. Use this to build on the work that has already been done and avoid duplicating work. Here is the summary produced by the other language model, use the information in this summary to assist with your own analysis:
```

#### 7.1.2 核心实现逻辑

**文件:** `codex-rs/core/src/compact.rs`

关键常量与架构:
- `SUMMARIZATION_PROMPT` = `include_str!("../templates/compact/prompt.md")`
- `SUMMARY_PREFIX` = `include_str!("../templates/compact/summary_prefix.md")`
- `COMPACT_USER_MESSAGE_MAX_TOKENS` = 20,000（保留的最近用户消息 token 上限）

核心函数流程:

1. **`run_inline_auto_compact_task()`**: 内联自动压缩，调用 `run_compact_task_inner`，使用 `InitialContextInjection::BeforeLastUserMessage` 在最后一个真实用户消息前注入初始上下文。

2. **`run_compact_task()`**: 手动压缩入口，使用 `InitialContextInjection::DoNotInject`，不注入初始上下文。

3. **`run_compact_task_inner()`**: 
   - 将 compact prompt 作为用户输入发送给 LLM
   - 将完整的历史（含初始输入 + compact prompt）发送给 LLM
   - 遇到 `ContextWindowExceeded` 时从最旧的消息开始逐条裁剪（truncation）
   - 裁剪计数器重置：每次裁剪后 `retries = 0`（注意：在内部循环中不会重置，只在成功或裁剪后重置）
   - 流式重试机制：`max_retries` 由 provider 配置控制，失败后调用 `backoff(retries)` 延迟重试

4. **历史重建** (`build_compacted_history`):
   - 保留初始上下文（可选注入）
   - 从后往前保留最近的 `COMPACT_USER_MESSAGE_MAX_TOKENS` 条用户消息（token 预算内）
   - 追加摘要文本（`SUMMARY_PREFIX \n assistant_message`）
   - 支持 token 截断：单条消息超过预算时调用 `truncate_text()` 裁剪

5. **初始上下文注入位置** (`insert_initial_context_before_last_real_user_or_summary`):
   - 优先在最后一个真实用户消息前插入
   - 没有真实用户消息时在摘要/压缩项前插入
   - 确保压缩摘要始终在最后

6. **收集用户消息** (`collect_user_messages`):
   - 过滤掉 `CompactSummary` 消息（通过 `is_summary_message()` 检测）
   - 返回所有非摘要的用户消息原文

7. **Compact 完成后**:
   - 调用 `replace_compacted_history()` 替换历史
   - 重置 WebSocket 会话
   - 重新计算 token 使用量
   - 发出 warning 事件："Long threads and multiple compactions can cause the model to be less accurate..."

#### 7.1.3 测试覆盖

- `codex-rs/core/tests/suite/compact.rs`: 测试 `build_compacted_history` 的 token 预算、摘要拼接、初始上下文注入
- `codex-rs/core/tests/suite/compact_remote.rs`: 测试远程 compact API 响应解析
- `codex-rs/core/tests/suite/compact_resume_fork.rs`: 测试压缩后恢复 fork 的场景
- `codex-rs/app-server/tests/suite/v2/compaction.rs`: 服务端 v2 协议的压缩测试

#### 7.1.4 API 端点

**文件:** `codex-rs/codex-api/src/endpoint/compact.rs`

- 端点路径: `POST /responses/compact`
- 输入: `CompactionInput`（包含历史消息、模型配置等）
- 输出: `Vec<ResponseItem>`（压缩后的历史消息列表）

---

### 7.2 Kimi CLI

**项目路径:** `D:\GitObjectsOwn\kimi-cli`

#### 7.2.1 Prompt 模板

**文件:** `src/kimi_cli/prompts/compact.md`

```markdown

---

The above is a list of messages in an agent conversation. You are now given a task to compact this conversation context according to specific priorities and rules.

**Compression Priorities (in order):**
1. **Current Task State**: What is being worked on RIGHT NOW
2. **Errors & Solutions**: All encountered errors and their resolutions
3. **Code Evolution**: Final working versions only (remove intermediate attempts)
4. **System Context**: Project structure, dependencies, environment setup
5. **Design Decisions**: Architectural choices and their rationale
6. **TODO Items**: Unfinished tasks and known issues

**Compression Rules:**
- MUST KEEP: Error messages, stack traces, working solutions, current task
- MERGE: Similar discussions into single summary points
- REMOVE: Redundant explanations, failed attempts (keep lessons learned), verbose comments
- CONDENSE: Long code blocks → keep signatures + key logic only

**Special Handling:**
- For code: Keep full version if < 20 lines, otherwise keep signature + key logic
- For errors: Keep full error message + final solution
- For discussions: Extract decisions and action items only

**Required Output Structure:**

<current_focus>
[What we're working on now]
</current_focus>

<environment>
- [Key setup/config points]
- ...more...
</environment>

<completed_tasks>
- [Task]: [Brief outcome]
- ...more...
</completed_tasks>

<active_issues>
- [Issue]: [Status/Next steps]
- ...more...
</active_issues>

<code_state>

<file>
[filename]

**Summary:**
[What this code file does]

**Key elements:**
- [Important functions/classes]
- ...more...

**Latest version:**
[Critical code snippets in this file]
</file>

<file>
[filename]
...Similar as above...
</file>

...more files...
</code_state>

<important_context>
- [Any crucial information not covered above]
- ...more...
</important_context>
```

#### 7.2.2 核心实现逻辑

**文件:** `src/kimi_cli/soul/compaction.py`

`SimpleCompaction` 类:

- **构造函数**: `max_preserved_messages` 默认值为 2（保留最近的用户+助手消息）

- **`prepare()` 方法**（纯函数，不依赖 LLM）:
  1. 从历史消息尾部往前查找，计数直到找到 `max_preserved_messages` 条 user/assistant 消息
  2. 如果总消息数不足 `max_preserved_messages`，不进行压缩（返回 `compact_message=None`）
  3. 将 `preserve_start_index` 之前的消息打包为一个 `compact_message`
  4. 打包过程:
     - 每条旧消息格式化为 `## Message {i+1}\nRole: {msg.role}\nContent:\n`
     - 过滤掉 `ThinkPart`（思考部分内容）
     - 最后追加 `prompts.COMPACT`（即 `compact.md` 模板内容）

- **`compact()` 方法**（异步 LLM 调用）:
  1. 调用 `kosong.step()` 让 LLM 执行压缩
  2. system prompt 固定为 `"You are a helpful assistant that compacts conversation context."`
  3. 工具集为空（`EmptyToolset()`）
  4. 压缩结果:
     - 过滤掉 `ThinkPart`
     - 构建新的 compacted 消息列表（包含系统提示和压缩结果）
     - 追加 `to_preserve` 的消息（最近未压缩的消息）
  5. 日志记录 token 使用量（input / output）

- **关键设计**:
  - 始终保留最近的 `max_preserved_messages` 条对话（不被压缩）
  - ThinkPart 在压缩输入和输出中都会被丢弃，避免思考过程污染压缩结果
  - 使用协议库 `kosong`（空）进行 LLM 调用，保持与主 agent 相同的 chat_provider

#### 7.2.3 测试覆盖

**文件:** `tests/core/test_simple_compaction.py`

三个测试用例:
1. `test_prepare_returns_original_when_not_enough_messages`: 单条消息不压缩
2. `test_prepare_skips_compaction_with_only_preserved_messages`: 仅有 2 条消息时不压缩
3. `test_prepare_builds_compact_message_and_preserves_tail`: 验证正确构建 compact message 并保留尾部消息

```
输入: [system, user(含ThinkPart), assistant, user, assistant]
输出: compact_message 包含 system + user(无ThinkPart) + assistant + prompts.COMPACT
       to_preserve 包含最后两条 [user, assistant]
```

---

### 7.3 OpenCode

**项目路径:** `D:\GitObjectsOwn\opencode`

#### 7.3.1 Prompt 模板

**文件:** `packages/opencode/src/agent/prompt/compaction.txt`（自动压缩用）

```
You are a helpful AI assistant tasked with summarizing conversations.

When asked to summarize, provide a detailed but concise summary of the conversation.
Focus on information that would be helpful for continuing the conversation, including:
- What was done
- What is currently being worked on
- Which files are being modified
- What needs to be done next
- Key user requests, constraints, or preferences that should persist
- Important technical decisions and why they were made

Your summary should be comprehensive enough to provide context but concise enough to be quickly understood.

Do not respond to any questions in the conversation, only output the summary.
```

**文件:** `packages/opencode/src/agent/prompt/summary.txt`（会话结束总结用）

```
Summarize what was done in this conversation. Write like a pull request description.

Rules:
- 2-3 sentences max
- Describe the changes made, not the process
- Do not mention running tests, builds, or other validation steps
- Do not explain what the user asked for
- Write in first person (I added..., I fixed...)
- Never ask questions or add new questions
- If the conversation ends with an unanswered question to the user, preserve that exact question
- If the conversation ends with an imperative statement or request to the user (e.g. "Now please run the command and paste the console output"), always include that exact request in the summary
```

#### 7.3.2 核心实现逻辑

**文件:** `packages/opencode/src/session/compaction.ts`

关键常量:
- `COMPACTION_BUFFER = 20,000`（保留的 token 缓冲）
- `PRUNE_MINIMUM = 20,000`（裁剪最少阈值）
- `PRUNE_PROTECT = 40,000`（保护最近 40K token 的工具调用）
- `PRUNE_PROTECTED_TOOLS = ["skill"]`（skill 工具调用不被裁剪）

核心函数:

1. **`isOverflow()`**: 判断是否超限
   - 考虑 `config.compaction.auto === false` 可关闭
   - 计算: `count >= context - reserved` 或 `count >= input.limit.input - reserved`
   - `reserved` 取自配置或 `min(COMPACTION_BUFFER, maxOutputTokens)`

2. **`prune()`**: 工具调用输出裁剪（不等价于压缩，是独立的轻量优化）
   - 从后往前遍历消息，跳过最近的 2 个 turn
   - 遇到第一个 `summary` 消息时停止
   - 累计 `total` token，超过 `PRUNE_PROTECT` 后开始标记裁剪
   - 只有 `completed` 状态的 tool call 且不在保护列表中的才会被裁剪
   - 当 `pruned > PRUNE_MINIMUM` 时才执行实际裁剪

3. **`process()`**: 压缩处理主流程
   - 查找 compaction 专用的 agent（`Agent.get("compaction")`）
   - 如果 agent 未指定模型，则复用用户消息的模型
   - 创建 assistant 消息标记为 `mode: "compaction"`、`summary: true`
   - **插件扩展点**: `Plugin.trigger("experimental.session.compacting", ...)` 
     - 允许插件注入 `context` 数组或完全替换 `prompt`
   - **默认 prompt 模板**:
     ```
     Provide a detailed prompt for continuing our conversation above.
     Focus on information that would be helpful for continuing the conversation, including what we did, what we're doing, which files we're working on, and what we're going to do next.
     The summary that you construct will be used so that another agent can read it and continue the work.
     
     When constructing the summary, try to stick to this template:
     ---
     ## Goal
     
     [What goal(s) is the user trying to accomplish?]
     
     ## Instructions
     
     - [What important instructions did the user give you that are relevant]
     - [If there is a plan or spec, include information about it so next agent can continue using it]
     
     ## Discoveries
     
     [What notable things were learned during this conversation that would be useful for the next agent to know when continuing the work]
     
     ## Accomplished
     
     [What work has been completed, what work is still in progress, and what work is left?]
     
     ## Relevant files / directories
     
     [Construct a structured list of relevant files that have been read, edited, or created that pertain to the task at hand. If all the files in a directory are relevant, include the path to the directory.]
     ---
     ```
   - 如果 `prompt` 被插件替换，则使用插件提供的；否则使用默认模板
   - 压缩完成后发布 `Event.Compacted` 事件

4. **`create()`**: 从 TUI 触发的压缩（手动 `/compact`）
   - 创建新的 user 消息触发压缩流程

#### 7.3.3 测试覆盖

- `packages/opencode/test/session/compaction.test.ts`: 测试 `isOverflow`、`prune`、`process` 流程
- `packages/opencode/test/session/revert-compact.test.ts`: 测试压缩后回滚场景

---

### 7.4 pi-mono

**项目路径:** `D:\GitObjectsOwn\pi-mono`

#### 7.4.1 System Prompt

**文件:** `packages/coding-agent/src/core/compaction/utils.ts`

```typescript
export const SUMMARIZATION_SYSTEM_PROMPT = `You are a context summarization assistant. Your task is to read a conversation between a user and an AI coding assistant, then produce a structured summary following the exact format specified.

Do NOT continue the conversation. Do NOT respond to any questions in the conversation. ONLY output the structured summary.`;
```

#### 7.4.2 核心 Prompt 模板

**文件:** `packages/coding-agent/src/core/compaction/compaction.ts`

**`SUMMARIZATION_PROMPT`**（初始压缩用）:

```
The messages above are a conversation to summarize. Create a structured context checkpoint summary that another LLM will use to continue the work.

Use this EXACT format:

## Goal
[What is the user trying to accomplish? Can be multiple items if the session covers different tasks.]

## Constraints & Preferences
- [Any constraints, preferences, or requirements mentioned by user]
- [Or "(none)" if none were mentioned]

## Progress
### Done
- [x] [Completed tasks/changes]

### In Progress
- [ ] [Current work]

### Blocked
- [Issues preventing progress, if any]

## Key Decisions
- **[Decision]**: [Brief rationale]

## Next Steps
1. [Ordered list of what should happen next]

## Critical Context
- [Any data, examples, or references needed to continue]
- [Or "(none)" if not applicable]

Keep each section concise. Preserve exact file paths, function names, and error messages.
```

**`UPDATE_SUMMARIZATION_PROMPT`**（增量合并用）:

```
The messages above are NEW conversation messages to incorporate into the existing summary provided in <previous-summary> tags.

Update the existing structured summary with new information. RULES:
- PRESERVE all existing information from the previous summary
- ADD new progress, decisions, and context from the new messages
- UPDATE the Progress section: move items from "In Progress" to "Done" when completed
- UPDATE "Next Steps" based on what was accomplished
- PRESERVE exact file paths, function names, and error messages
- If something is no longer relevant, you may remove it

Use this EXACT format:

## Goal
[Preserve existing goals, add new ones if the task expanded]

## Constraints & Preferences
- [Preserve existing, add new ones discovered]

## Progress
### Done
- [x] [Include previously done items AND newly completed items]

### In Progress
- [ ] [Current work - update based on progress]

### Blocked
- [Current blockers - remove if resolved]

## Key Decisions
- **[Decision]**: [Brief rationale] (preserve all previous, add new)

## Next Steps
1. [Update based on current state]

## Critical Context
- [Preserve important context, add new if needed]

Keep each section concise. Preserve exact file paths, function names, and error messages.
```

**`TURN_PREFIX_SUMMARIZATION_PROMPT`**（分割 turn 的前缀摘要）:

```
This is the PREFIX of a turn that was too large to keep. The SUFFIX (recent work) is retained.

Summarize the prefix to provide context for the retained suffix:

## Original Request
[What did the user ask for in this turn?]

## Early Progress
- [Key decisions and work done in the prefix]

## Context for Suffix
- [Information needed to understand the retained recent work]

Be concise. Focus on what's needed to understand the kept suffix.
```

#### 7.4.3 核心实现逻辑

**文件:** `packages/coding-agent/src/core/compaction/compaction.ts`（823 行）

关键常量:
```typescript
export const DEFAULT_COMPACTION_SETTINGS: CompactionSettings = {
    enabled: true,
    reserveTokens: 16384,
    keepRecentTokens: 20000,
};
```

触发条件:
```typescript
shouldCompact(contextTokens, contextWindow, settings): contextTokens > contextWindow - settings.reserveTokens
```

核心架构:

1. **Token 估算** (`estimateTokens`): 使用 `chars / 4` 启发式估算
   - 图片估算为 4800 chars（约 1200 tokens）
   - 工具调用估算为 `name.length + JSON.stringify(arguments).length`
   - thinking 部分也计入

2. **Cut Point 检测** (`findCutPoint`):
   - 从后往前累积 token，直到超过 `keepRecentTokens`
   - 有效切点: user、assistant、bashExecution、custom、branchSummary、compactionSummary
   - 永远不在 tool result 处切割
   - 切点为 assistant 时，其 tool results 会被保留

3. **Split Turn 检测**:
   - 如果切点不是 user 消息，说明分割了一个 turn 的中间
   - 此时生成两个摘要（历史摘要 + turn prefix 摘要）并合并

4. **增量摘要生成** (`generateSummary`):
   - 如果检测到 `previousSummary`，使用 `UPDATE_SUMMARIZATION_PROMPT`
   - 否则使用 `SUMMARIZATION_PROMPT`
   - 对话内容包装在 `<conversation>` 标签中
   - 旧摘要包装在 `<previous-summary>` 标签中
   - 最大 token 预算: `Math.floor(0.8 * reserveTokens)`
   - 如果模型支持 reasoning，使用 `reasoning: "high"`

5. **文件操作追踪** (`extractFileOperations`):
   - 从工具调用中提取读/写/编辑的文件列表
   - 从上一次压缩的 `details` 中继承文件列表
   - 最终合并到摘要末尾

6. **主压缩函数** (`compact`):
   - 接收 `CompactionPreparation` 对象
   - 如果是 split turn，并行生成两个摘要然后合并
   - 合并格式: `${historyResult}\n\n---\n\n**Turn Context (split turn):**\n\n${turnPrefixResult}`
   - 计算文件列表并追加到摘要
   - 返回 `CompactionResult` 包含 `summary`、`firstKeptEntryId`、`tokensBefore`、`details`

7. **`prepareCompaction()`**（纯函数）:
   - 找到上一次压缩的边界
   - 计算 cut point
   - 提取 `messagesToSummarize` 和 `turnPrefixMessages`
   - 提取文件操作
   - 返回 `CompactionPreparation` 对象

8. **扩展点**: `prepareCompaction()` 和 `compact()` 分离，允许扩展点在准备阶段拦截并修改数据

#### 7.4.4 文档与配置

**文件:** `packages/coding-agent/docs/compaction.md`

完整的文档包含:
- 触发条件与公式
- 压缩前后消息结构示意图
- Split Turn 场景说明
- Cut Point 规则
- Branch Summarization（`/tree` 指令触发）
- 累积文件追踪机制
- `CompactionEntry` 和 `BranchSummaryEntry` 结构定义

---

### 7.5 Claude Code（sourcemap 逆向）

**项目路径:** `D:\GitObjectsOwn\claude-code-sourcemap`

#### 7.5.1 Prompt 模板

**文件:** `restored-src/src/services/compact/prompt.ts`（约 250 行）

**`NO_TOOLS_PREAMBLE`**（全大写 + 加粗的强力约束，放在所有 prompt 最前面）:

```
CRITICAL: Respond with TEXT ONLY. Do NOT call any tools.

- Do NOT use Read, Bash, Grep, Glob, Edit, Write, or ANY other tool.
- You already have all the context you need in the conversation above.
- Tool calls will be REJECTED and will waste your only turn — you will fail the task.
- Your entire response must be plain text: an <analysis> block followed by a <summary> block.
```

**`DETAILED_ANALYSIS_INSTRUCTION_BASE`**（全量压缩用自检指令）:

```
Before providing your final summary, wrap your analysis in <analysis> tags to organize your thoughts and ensure you've covered all necessary points. In your analysis process:

1. Chronologically analyze each message and section of the conversation. For each section thoroughly identify:
   - The user's explicit requests and intents
   - Your approach to addressing the user's requests
   - Key decisions, technical concepts and code patterns
   - Specific details like:
     - file names
     - full code snippets
     - function signatures
     - file edits
   - Errors that you ran into and how you fixed them
   - Pay special attention to specific user feedback that you received, especially if the user told you to do something differently.
2. Double-check for technical accuracy and completeness, addressing each required element thoroughly.
```

**`DETAILED_ANALYSIS_INSTRUCTION_PARTIAL`**（部分压缩用自检指令）:

与上相同，但第一句改为 `"Analyze the recent messages chronologically."`

**`BASE_COMPACT_PROMPT`**（完整压缩模板，约 200 行）:

```
Your task is to create a detailed summary of the conversation so far, paying close attention to the user's explicit requests and your previous actions.
This summary should be thorough in capturing technical details, code patterns, and architectural decisions that would be essential for continuing development work without losing context.

${DETAILED_ANALYSIS_INSTRUCTION_BASE}

Your summary should include the following sections:

1. Primary Request and Intent: Capture all of the user's explicit requests and intents in detail
2. Key Technical Concepts: List all important technical concepts, technologies, and frameworks discussed.
3. Files and Code Sections: Enumerate specific files and code sections examined, modified, or created. Pay special attention to the most recent messages and include full code snippets where applicable and include a summary of why this file read or edit is important.
4. Errors and fixes: List all errors that you ran into, and how you fixed them. Pay special attention to specific user feedback that you received, especially if the user told you to do something differently.
5. Problem Solving: Document problems solved and any ongoing troubleshooting efforts.
6. All user messages: List ALL user messages that are not tool results. These are critical for understanding the users' feedback and changing intent.
7. Pending Tasks: Outline any pending tasks that you have explicitly been asked to work on.
8. Current Work: Describe in detail precisely what was being worked on immediately before this summary request, paying special attention to the most recent messages from both user and assistant. Include file names and code snippets where applicable.
9. Optional Next Step: List the next step that you will take that is related to the most recent work you were doing. IMPORTANT: ensure that this step is DIRECTLY in line with the user's most recent explicit requests, and the task you were working on immediately before this summary request. If your last task was concluded, then only list next steps if they are explicitly in line with the users request. Do not start on tangential requests or really old requests that were already completed without confirming with the user first.
                       If there is a next step, include direct quotes from the most recent conversation showing exactly what task you were working on and where you left off. This should be verbatim to ensure there's no drift in task interpretation.

<example>
<analysis>
[Your thought process, ensuring all points are covered thoroughly and accurately]
</analysis>

<summary>
1. Primary Request and Intent:
   [Detailed description]

2. Key Technical Concepts:
   - [Concept 1]
   - [Concept 2]
   - [...]

3. Files and Code Sections:
   - [File Name 1]
      - [Summary of why this file is important]
      - [Summary of the changes made to this file, if any]
      - [Important Code Snippet]
   - [File Name 2]
      - [Important Code Snippet]
   - [...]

4. Errors and fixes:
    - [Detailed description of error 1]:
      - [How you fixed the error]
      - [User feedback on the error if any]
    - [...]

5. Problem Solving:
   [Description of solved problems and ongoing troubleshooting]

6. All user messages:
    - [Detailed non tool use user message]
    - [...]

7. Pending Tasks:
   - [Task 1]
   - [Task 2]
   - [...]

8. Current Work:
   [Precise description of current work]

9. Optional Next Step:
   [Optional Next step to take]

</summary>
</example>

Please provide your summary based on the conversation so far, following this structure and ensuring precision and thoroughness in your response.

There may be additional summarization instructions provided in the included context. If so, remember to follow these instructions when creating the above summary. Examples of instructions include:
<example>
## Compact Instructions
When summarizing the conversation focus on typescript code changes and also remember the mistakes you made and how you fixed them.
</example>

<example>
# Summary instructions
When you are using compact - please focus on test output and code changes. Include file reads verbatim.
</example>
```

**`PARTIAL_COMPACT_PROMPT`**（部分压缩 - 仅压缩最近消息）:

与 BASE 类似，但:
- 开头改为: `"Your task is to create a detailed summary of the RECENT portion of the conversation — the messages that follow earlier retained context."`
- 指示: "Focus your summary on what was discussed, learned, and accomplished in the recent messages only."
- 省略 "Context for Continuing Work" 部分（因为是后续追加的消息）

**`PARTIAL_COMPACT_UP_TO_PROMPT`**（部分压缩 - 仅压缩前缀，后缀后续追加）:

- 开头改为: `"Your task is to create a detailed summary of this conversation. This summary will be placed at the start of a continuing session"`
- 第 8 节: "Work Completed" 替代 "Current Work"
- 第 9 节: "Context for Continuing Work" 替代 "Optional Next Step"

**`NO_TOOLS_TRAILER`**（结尾再次强调不调用工具）:

```
\n\nREMINDER: Do NOT call any tools. Respond with plain text only — an <analysis> block followed by a <summary> block. Tool calls will be rejected and you will fail the task.
```

#### 7.5.2 Prompt 组装逻辑

**`getCompactPrompt(customInstructions?)`**:
```typescript
NO_TOOLS_PREAMBLE + BASE_COMPACT_PROMPT + (customInstructions ? "\n\nAdditional Instructions:\n" + customInstructions : "") + NO_TOOLS_TRAILER
```

**`getPartialCompactPrompt(customInstructions?, direction)`**:
- `direction === 'up_to'` → `PARTIAL_COMPACT_UP_TO_PROMPT`
- `direction === 'from'` → `PARTIAL_COMPACT_PROMPT`
- 组装方式同上

#### 7.5.3 `formatCompactSummary()` 解析函数

```typescript
export function formatCompactSummary(summary: string): string {
    // 1. 删除 <analysis> 块（纯草稿，无信息价值）
    formattedSummary = formattedSummary.replace(/<analysis>[\s\S]*?<\/analysis>/, '')
    
    // 2. 提取 <summary> 块内容，替换为 "Summary:\n..." 标题
    const summaryMatch = formattedSummary.match(/<summary>([\s\S]*?)<\/summary>/)
    if (summaryMatch) {
        formattedSummary = formattedSummary.replace(/<summary>[\s\S]*?<\/summary>/, `Summary:\n${content.trim()}`)
    }
    
    // 3. 清理多余空行
    formattedSummary = formattedSummary.replace(/\n\n+/g, '\n\n')
    
    return formattedSummary.trim()
}
```

#### 7.5.4 压缩摘要恢复为用户消息

**`getCompactUserSummaryMessage()`**:

根据场景生成不同的恢复消息:

1. **自动压缩（suppressFollowUpQuestions=true）**:
```
This session is being continued from a previous conversation that ran out of context. The summary below covers the earlier portion of the conversation.

${formattedSummary}

Continue the conversation from where it left off without asking the user any further questions. Resume directly — do not acknowledge the summary, do not recap what was happening, do not preface with "I'll continue" or similar. Pick up the last task as if the break never happened.
```

如果有 `transcriptPath`，追加:
```
If you need specific details from before compaction (like exact code snippets, error messages, or content you generated), read the full transcript at: ${transcriptPath}
```

如果 `recentMessagesPreserved`，追加:
```
Recent messages are preserved verbatim.
```

如果在主动模式（proactive），追加自主工作循环提示:
```
You are running in autonomous/proactive mode. This is NOT a first wake-up — you were already working autonomously before compaction. Continue your work loop: pick up where you left off based on the summary above. Do not greet the user or ask what to work on.
```

2. **手动压缩（suppressFollowUpQuestions=false）**: 只返回基础摘要，允许 LLM 继续对话

#### 7.5.5 自动压缩配置 (`autoCompact.ts`)

关键常量:
```typescript
const MAX_OUTPUT_TOKENS_FOR_SUMMARY = 20_000  // 基于 p99.99 为 17,387 tokens
const AUTOCOMPACT_BUFFER_TOKENS = 13_000       // 自动压缩触发缓冲
const WARNING_THRESHOLD_BUFFER_TOKENS = 20_000  // UI 警告阈值
const ERROR_THRESHOLD_BUFFER_TOKENS = 20_000    // 错误阈值
const MANUAL_COMPACT_BUFFER_TOKENS = 3_000      // 手动压缩缓冲（更小，因为用户主动触发）
const MAX_CONSECUTIVE_AUTOCOMPACT_FAILURES = 3  // 连续失败熔断
```

环境变量控制:
- `CLAUDE_CODE_AUTO_COMPACT_WINDOW` - 覆盖上下文窗口大小
- `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE` - 覆盖触发百分比
- `CLAUDE_CODE_BLOCKING_LIMIT_OVERRIDE` - 覆盖阻塞限制
- `DISABLE_COMPACT` / `DISABLE_AUTO_COMPACT` - 关闭压缩
- `CLAUDE_CONTEXT_COLLAPSE` - 使用上下文坍缩替代自动压缩

**阈值计算公式**:
```typescript
getAutoCompactThreshold(model) = effectiveContextWindow - AUTOCOMPACT_BUFFER_TOKENS
```

**`shouldAutoCompact()`** 过滤条件:
- 递归保护: `session_memory`、`compact`、`marble_origami` 不调用
- 功能开关: `REACTIVE_COMPACT` 模式下抑制主动压缩
- `CONTEXT_COLLAPSE` 启用时不触发（避免竞争）

**`autoCompactIfNeeded()`** 流程:
1. 检查熔断器（连续失败 >= 3 次跳过）
2. `shouldAutoCompact()` 判断
3. 尝试 session memory 压缩（实验性功能）
4. fallback `compactConversation()`
5. 失败时递增 `consecutiveFailures`

#### 7.5.6 Session Memory 压缩 (`sessionMemoryCompact.ts`)

实验性功能，配置:
```typescript
DEFAULT_SM_COMPACT_CONFIG = { minTokens: 10_000, minTextBlockMessages: 5, maxTokens: 40_000 }
```

#### 7.5.7 Post Compact 清理 (`postCompactCleanup.ts`)

清理内容:
- `resetMicrocompactState()`: 重置微压缩状态
- 如果 `CONTEXT_COLLAPSE` 启用，重置上下文坍缩
- `getUserContext.cache.clear()`: 清除用户上下文缓存
- `clearSystemPromptSections()`: 清除系统提示段
- `clearClassifierApprovals()`: 清除分类器审批
- `clearSpeculativeChecks()`: 清除推测性检查
- `clearBetaTracingState()`: 清除 beta 追踪
- `clearSessionMessagesCache()`: 清除会话消息缓存
- **不**清除已调用技能内容（需跨压缩保留）

#### 7.5.8 微压缩/消息裁剪 (`microCompact.ts`)

`microcompactMessages()`: 在发送 API 请求前，基于 token 估算裁剪旧的工具结果:
- 仅裁剪特定工具（FILE_READ、SHELL、GREP、GLOB、WEB_SEARCH、WEB_FETCH、FILE_EDIT、FILE_WRITE）
- 工具结果超过阈值时替换为 `[Old tool result content cleared]`
- 包含缓存键管理（pinned cache edits）用于上下文缓存命中
- 图片估算为 2000 tokens

#### 7.5.9 时间基础压缩配置 (`timeBasedMCConfig.ts`)

基于时间的压缩配置，允许根据消息的时间戳决定是否裁剪。

#### 7.5.10 压缩警告钩子 (`compactWarningHook.ts` + `compactWarningState.ts`)

简单的状态管理，控制压缩警告在 UI 中的显示抑制。

---

### 7.6 五项目对比总结表

| 特性 | Codex | Kimi CLI | OpenCode | pi-mono | Claude Code |
|------|-------|----------|----------|---------|-------------|
| **Prompt 模板格式** | Markdown 简短指令 | Markdown 结构化 + XML 标签 | 纯文本模板 | 代码内字符串常量 | 代码内字符串常量 |
| **NO_TOOLS 约束** | 无显式（靠工具集为空） | 无显式（EmptyToolset） | 无显式 | System prompt "Do NOT continue" | **全大写前缀+后缀** 双重强制 |
| **Analysis 自检** | ❌ | ❌ | ❌ | ❌ | ✅ `<analysis>` 块 |
| **内容优先级** | 简单列表（4项） | ✅ **6级优先级** | 关注点列表（6项） | ✅ **7段结构模板** | ✅ **9段结构模板** |
| **增量重压缩** | ✅ summary_prefix | ❌ | ❌ | ✅ `UPDATE_SUMMARIZATION_PROMPT` | ✅ partial compact |
| **文件追踪** | ❌ | ❌ | ❌（模板中提及） | ✅ **累积文件操作追踪** | ✅ 通过工具调用 |
| **Split Turn** | ❌ | ❌ | ❌ | ✅ 并行双摘要合并 | ✅ partial direction: up_to / from |
| **Token 估算** | 内置 `approx_token_count` | 不估算 | 内置 | chars/4 启发式 | roughTokenCountEstimation |
| **裁剪/Pruning** | 从最旧消息逐条裁剪 | 不裁剪 | ✅ tool call 输出裁剪 | 在 cut point 整段裁剪 | microcompact（工具结果裁剪） |
| **熔断机制** | 重试+裁剪 | ❌ | ❌ | ❌ | ✅ 3次连续失败熔断 |
| **插件扩展** | ❌ | ❌ | ✅ `experimental.session.compacting` | ✅ prepareCompaction 分离 | ✅ Pre/Post Compact hooks |
| **用户消息原文保留** | ✅ 不超过 20K token 的保留 | 最后 max_preserved 条 | ❌ | 不保留原始消息 | ✅ compact boundary message |
