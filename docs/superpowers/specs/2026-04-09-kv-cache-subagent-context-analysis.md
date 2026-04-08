# KV 缓存与子智能体上下文管理 — 问题分析

> 分析日期：2026-04-09
> 分支：003-subagent-child-sessions

## 一、当前架构概览

### 1.1 Prompt 分层构建（已完成）

```
Stable 层（永不过期）     → Identity + Environment
SemiStable 层（5min TTL） → AGENTS.md + CapabilityPrompt + AgentProfile + SkillSummary
Dynamic 层（不缓存）      → WorkflowExamples
```

- `cache_boundary` 仅标记在层边界：Stable→SemiStable、SemiStable→Dynamic、prompt 尾部
- Anthropic provider 在 boundary 处放置 `cache_control: { type: "ephemeral" }`
- 消息缓存深度 = 3（最近 3 条消息的最后一个 content block 标记 ephemeral）
- 工具定义仅最后一个 tool 标记 cache_control

### 1.2 子 Agent 生命周期

```
父 Turn 执行 spawnAgent
  → resolve_parent_execution()   获取父会话快照
  → prepare_child()              准备子 Agent 执行配置
  → spawn_child()                注册 AgentControl + 创建 session + 构建 event sink
  → emit_child_started()         持久化 SubRunStarted + Notification(Started)
  → [后台 tokio task]
      run_turn_with_agent_context_and_owner()  ← 子 Agent 走完整的 agent loop
      finalize_child_execution()               ← 持久化 SubRunFinished + Notification(Delivered)
      reactivate_parent_agent_if_idle()        ← 向父会话注入 ReactivationPrompt
```

### 1.3 上下文传递方式

`resolve_context_snapshot()` 将以下内容拼成一个字符串作为子 Agent 的 User 消息：

```
# Task
{prompt}

# Context
{context}                     ← 可选补充

# Parent Compact Summary
{summary}                     ← 父会话的压缩摘要

# Recent Tail
- user: ...
- assistant: ...
- tool[id]: ...
```

### 1.4 事件投影与隔离

- `AgentStateProjector` 过滤 `invocation_kind == SubRun` 的事件，不进入父的 projected state
- SharedSession：子事件写入父 session 日志但被投影过滤
- IndependentSession：子事件写入独立 session

---

## 二、已识别的问题

### 问题 1：子 Agent 冷启动 — 全量 KV 缓存重建

**严重程度：高**

`build_child_agent_state()` 创建一个只有 1 条 User 消息的空 `AgentState`。子 Agent 进入 agent loop 后走完整的 `LayeredPromptBuilder` 构建 Stable + SemiStable + Dynamic 三层 system prompt。

- 每个子 Agent 首个请求都需要 Anthropic 服务端重建整个 KV cache（写入 `cache_creation_input_tokens`）
- 即使两个子 Agent 使用相同 profile（如 explore），它们的 Stable 层内容完全相同，也无法共享缓存
- 预计每个子 Agent 冷启动消耗数千 cache_creation tokens

**影响文件：**
- `crates/runtime-execution/src/prep.rs:216` — `build_child_agent_state()`
- `crates/runtime-agent-loop/src/agent_loop/turn_runner.rs:173` — `build_plan()`

### 问题 2：ReactivationPrompt 破坏父会话缓存连续性

**严重程度：中**

子 Agent 完成后，`reactivate_parent_agent_if_idle()` 通过 `submit_prompt_with_origin()` 向父会话注入一条 `UserMessageOrigin::ReactivationPrompt` 消息，触发新的 LLM 调用。

- 这条消息出现在父会话的对话流中，破坏了消息缓存的连续性
- EventTranslator 虽然过滤了 ReactivationPrompt 不作为 UserMessage 事件回放给前端，但它仍然进入 LLM 的消息列表
- 每次子 Agent 交付都导致父会话的 message cache 重新 build

**影响文件：**
- `crates/runtime-agent-loop/src/subagent.rs:201` — `build_parent_reactivation_prompt()`
- `crates/runtime/src/service/execution/mod.rs:137` — `reactivate_parent_agent_if_idle()`

### 问题 3：上下文继承是粗粒度文本拼接

**严重程度：中**

`resolve_context_snapshot()` 将父会话的 compact summary 和 recent tail 直接拼成一段文本，作为子 Agent 的唯一 User 消息。

- 子 Agent 无法区分"任务描述"和"从父继承的背景信息"
- 拼接文本作为单条 User 消息，没有 cache 断点
- Recent tail 的格式（`- user: ...`、`- assistant: ...`）丢失了结构信息
- 截断策略粗糙（`single_line` 200 字符限制），可能丢失关键上下文

**影响文件：**
- `crates/runtime-execution/src/context.rs:19` — `resolve_context_snapshot()`

### 问题 4：SharedSession 模式下无独立缓存空间

**严重程度：中低**

SharedSession 模式下子 Agent 的事件写入父 session 日志，但通过 `AgentStateProjector` 被过滤不进入父的 projected state。这导致：

- 子 Agent 的消息历史无法被自己的 prompt assembly 引用（因为它看到的是父的 conversation）
- 子 Agent 实际上每次 step 都是从 `build_child_agent_state()` 创建的只有初始任务的 state 开始
- 没有跨 step 的消息积累，缓存无法在 step 间复用

**影响文件：**
- `crates/core/src/projection/agent_state.rs:71` — `AgentStateProjector`
- `crates/runtime/src/service/execution/subagent.rs:264` — `build_child_agent_state()`

### 问题 5：无跨 Session 的缓存前缀共享

**严重程度：低（受限于 Anthropic API 设计）**

Anthropic 的 KV cache 是按请求维度绑定的，无法跨 session 共享。但以下优化方向可探索：

- 同 profile 的子 Agent 共享 Stable 层指纹，确认内容一致后至少减少 fingerprint 计算
- 将子 Agent 的 prompt 尽量与父保持同构（相同的 Stable 前缀），理论上 Anthropic 后端可能做 prefix dedup

---

## 三、待深入探索的方向

以下方向需要在下一步深入代码确认：

1. **子 Agent 的 agent loop 是否共享父的 PromptRuntime？** 如果共享，Stable 层缓存（内存层）可能已被预热
2. **子 Agent 的 conversation view 如何构建？** 是从空的 state.messages 开始还是有其他注入点？
3. **ReactivationPrompt 在 compose_messages 中是否被当作普通 User 消息？**
4. **子 Agent 执行多个 step 时，后续 step 是否能利用首个 step 建立的缓存？**
5. **compact 操作在子 Agent session 中的行为？** 是否会误压缩父传递的上下文？

---

## 四、关键文件索引

| 文件 | 职责 |
|------|------|
| `crates/runtime-prompt/src/layered_builder.rs` | 三层 prompt 构建 + 内存层缓存 |
| `crates/runtime-prompt/src/composer.rs` | 单层 contributor pipeline |
| `crates/runtime-agent-loop/src/request_assembler.rs` | PromptPlan → ModelRequest，标记 cache_boundary |
| `crates/runtime-agent-loop/src/prompt_runtime.rs` | PromptRuntime 入口，组装 contributor |
| `crates/runtime-llm/src/anthropic.rs` | Anthropic provider，放置 ephemeral cache_control |
| `crates/runtime-llm/src/cache_tracker.rs` | 缓存断裂检测（仅日志） |
| `crates/runtime-execution/src/prep.rs:216` | `build_child_agent_state()` — 子 Agent 初始状态 |
| `crates/runtime-execution/src/context.rs:19` | `resolve_context_snapshot()` — 上下文继承 |
| `crates/runtime/src/service/execution/subagent.rs` | 子 Agent 启动/结束流程 |
| `crates/runtime/src/service/execution/mod.rs:137` | 父会话 reactivation |
| `crates/core/src/projection/agent_state.rs` | 事件投影过滤 |
| `crates/runtime-agent-loop/src/subagent.rs` | ChildExecutionTracker + reactivation prompt |
