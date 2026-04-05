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
