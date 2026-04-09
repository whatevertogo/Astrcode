# 子智能体架构优化设计 — 存储分离 + 缓存优化 + 上下文改进

> 日期：2026-04-09
> 分支：003-subagent-child-sessions
> 状态：设计评审

---

## 一、问题总览

当前子智能体系统存在三个层面的问题，它们互相加剧：

```
存储混写 → 事件边界不清 → 投影过滤到处散落
   ↓                ↓
缓存无共享 → 子 Agent 冷启动全量重建 KV cache
   ↓                ↓
上下文拼接 → 父子之间信息传递粗糙 → ReactivationPrompt 破坏消息缓存
```

**影响量化（估算，每次子 Agent 调用）：**

| 浪费项 | 估算 tokens | 原因 |
|--------|------------|------|
| Stable 层重渲染 + cache_creation | 500-800 | LayerCache 不共享 |
| SemiStable 层重渲染 + cache_creation | 2000-4000 | LayerCache 不共享 |
| 父会话 message cache miss | 500-1500 | ReactivationPrompt 破坏缓存 |
| 上下文继承拼接 | 0（无额外消耗）但信息丢失 | single_line 200字符截断 |
| **每个子 Agent 总浪费** | **3000-6300** | — |
| **5 个子 Agent 累计** | **15000-31500** | — |

---

## 二、设计原则

### 原则 1：存储分离，视图聚合

主会话和子会话是不同的一致性边界（aggregate）。各自的 JSONL 文件只记录自己的事件。关系通过 ID 引用（`parent_session_id`、`sub_run_id`）。UI 层再聚合展示。

### 原则 2：渲染共享，请求独立

同一 `RuntimeService` 下的父子 Agent 共享 `LayerCache`（内存层的渲染结果缓存），避免重复渲染 Stable/SemiStable 层。但 Anthropic KV cache 是 per-request 绑定的，无法跨请求共享——这部分只能通过减少冗余来缓解。

### 原则 3：结构化传递，非文本拼接

父到子的上下文传递应该结构化：compact summary 和 recent tail 作为 system prompt block 注入，而非拼接到 User 消息中。子 Agent 能区分"任务"和"继承的背景"。

---

## 三、改动 1：存储分离 — IndependentSession 成为默认

### 3.1 现状

- `SharedSession`（默认）：子事件写入父的 JSONL，被 `AgentStateProjector` 过滤不投影
- `IndependentSession`（实验性）：子事件写入独立 JSONL，需要 `experimentalIndependentSession` flag
- 基础设施已完全就绪：`SessionStateEventSink`、`build_event_sinks()`、`resolve_existing_session_path()`

### 3.2 改动

**核心变更：翻转默认值。**

```
文件：crates/core/src/agent/mod.rs:287
- storage_mode: SubRunStorageMode::SharedSession,
+ storage_mode: SubRunStorageMode::IndependentSession,
```

**移除实验性 flag 守卫：**

```
文件：crates/runtime-execution/src/policy.rs:66-74
删除：
  if matches!(resolved.storage_mode, SubRunStorageMode::IndependentSession)
      && !resolve_agent_experimental_independent_session(runtime_config.agent.as_ref())
  {
      return Err(...)
  }
```

**简化 `should_project_into_session_state()`：**

```rust
// 文件：crates/core/src/projection/agent_state.rs
fn should_project_into_session_state(event: &StorageEvent) -> bool {
    match event.agent_context() {
        None => true,
        Some(agent) => agent.invocation_kind != Some(InvocationKind::SubRun),
    }
}
```

SharedSession 分支的 `matches!(agent.storage_mode, Some(IndependentSession))` 判断不再需要——所有 SubRun 事件都不投影到父 session，因为它们写入了子 session 自己的 JSONL。

### 3.3 父 session 中保留的事件

父 session 的 JSONL 只记录以下事件（原有的 `parent_event_sink` 已在处理这些）：

| 事件 | 内容 |
|------|------|
| `SubRunStarted` | 子运行元数据（profile、overrides、limits） |
| `SubRunFinished` | 子运行结果（summary、step_count、token 估算） |
| `ChildSessionNotification(Started)` | 通知前端子 Agent 已启动 |
| `ChildSessionNotification(Delivered/Failed/Closed)` | 通知前端子 Agent 已完成 |
| `ChildSessionNotification(Resumed)` | 通知前端子 Agent 已恢复 |

子 Agent 的内部事件（UserMessage、AssistantFinal、ToolCall、ToolResult、PromptMetrics、CompactApplied、TurnDone）全部写入子 session 自己的 JSONL。

### 3.4 目录结构（已有，无需改动）

```
~/.astrcode/projects/<project>/sessions/
    ├── {main-session-id}/
    │   ├── session-{id}.jsonl        ← 父会话事件（含 SubRunStarted/Finished）
    │   ├── active-turn.lock
    │   └── active-turn.json
    ├── {child-session-id-1}/
    │   └── session-{id}.jsonl        ← 子 Agent 1 的完整事件
    └── {child-session-id-2}/
        └── session-{id}.jsonl        ← 子 Agent 2 的完整事件
```

### 3.5 需要验证的边界情况

- `resume_child_session()`：恢复时需从子 session 加载事件重放 AgentState（当前实现已使用 `build_child_agent_state()` 重建空状态，可能需要改为从子 session 重放）
- `deliver_to_parent()`：子 Agent 向父投递消息时，事件走 `parent_event_sink`（已有）
- SSE 过滤：`SessionEventFilter` 的 `SubRunEventScope` 逻辑无需改动（它已基于 `SubRunDescriptor` 工作）
- `ExecutionLineageIndex`：从父 session 事件构建（只看到 SubRunStarted/Finished），跨 session 的 lineage 查询需要额外处理

### 3.6 迁移策略

- 不做数据迁移。旧的 SharedSession 数据保持不变
- 新会话默认使用 IndependentSession
- `SubRunStorageMode` 枚举保留 `SharedSession` 变体但标记 `#[deprecated]`
- 如果有用户依赖旧行为，可通过 agent profile override 回退

---

## 四、改动 2：缓存优化 — 共享 LayerCache

### 4.1 现状

```
build_scoped_agent_loop()           ← runtime/service/loop_factory.rs:97
  → build_agent_loop_from_parts()   ← loop_factory.rs:62
    → AgentLoop::from_capabilities_with_prompt_inputs()  ← agent_loop.rs:177
      → PromptRuntime::new()        ← prompt_runtime.rs:64
        → default_layered_builder() ← prompt_runtime.rs:147
          → LayeredPromptBuilder::new()
            → LayerCache::default() ← 全新空缓存
```

每个子 Agent 的 Step 0 都要重新渲染 Identity、Environment、AGENTS.md 等 Stable 层内容，即使父 Agent 已经渲染过完全相同的内容。

### 4.2 设计：注入共享 LayerCache

**核心思路：** `LayeredPromptBuilder` 的 `cache` 字段从 `Arc<Mutex<LayerCache>>` 改为接受外部注入。

#### 步骤 4.2.1：LayeredPromptBuilder 支持外部注入 cache

```rust
// 文件：crates/runtime-prompt/src/layered_builder.rs

impl LayeredPromptBuilder {
    pub fn with_shared_cache(mut self, cache: Arc<Mutex<LayerCache>>) -> Self {
        self.cache = cache;
        self
    }
}
```

#### 步骤 4.2.2：PromptRuntime 暴露 cache 引用

```rust
// 文件：crates/runtime-agent-loop/src/prompt_runtime.rs

impl PromptRuntime {
    /// 返回内部 LayeredPromptBuilder 的 LayerCache 引用，
    /// 供子 Agent 构建时注入以共享渲染缓存。
    pub(crate) fn layer_cache(&self) -> Option<Arc<Mutex<LayerCache>>> {
        match &self.backend {
            PromptBackend::Layered(builder) => Some(builder.cache()),
            PromptBackend::Composer(_) => None,
        }
    }
}
```

#### 步骤 4.2.3：AgentLoop 暴露 cache 引用

```rust
// 文件：crates/runtime-agent-loop/src/agent_loop.rs

impl AgentLoop {
    pub fn prompt_layer_cache(&self) -> Option<Arc<Mutex<LayerCache>>> {
        self.prompt.layer_cache()
    }
}
```

#### 步骤 4.2.4：子 Agent 构建时注入父的 cache

```rust
// 文件：crates/runtime/service/loop_factory.rs

pub(super) fn build_scoped_agent_loop(
    capabilities: CapabilityRouter,
    prompt_declarations: Vec<PromptDeclaration>,
    skill_catalog: Arc<SkillCatalog>,
    hook_handlers: Vec<Arc<dyn HookHandler>>,
    active_profile: &str,
    runtime_config: &crate::config::RuntimeConfig,
    deps: LoopRuntimeDeps,
    parent_layer_cache: Option<Arc<Mutex<LayerCache>>>,  // ← 新增
) -> Arc<AgentLoop> {
    // ... 构建 PromptRuntime 时注入 cache ...
}
```

调用方在 `subagent.rs` 的 `spawn_child()` 中传入父 AgentLoop 的 cache：

```rust
let parent_cache = parent_agent_loop.prompt_layer_cache();
let child_loop = build_scoped_agent_loop(
    /* ... */,
    parent_cache,
);
```

### 4.3 缓存命中行为

| 层 | 父 Step N 已渲染 | 子 Step 0 | 行为 |
|----|-----------------|-----------|------|
| Stable | Identity + Environment | 相同 working_dir | **命中缓存**，直接复用渲染结果 |
| SemiStable | AGENTS.md + Capability + ... | 相同 working_dir + 子集工具 | **可能命中**（AGENTS.md 相同，但 CapabilityPrompt 可能因工具子集不同而变化） |
| Dynamic | WorkflowExamples | 相同 | 不缓存（每次重新渲染） |

**关键：** Stable 层几乎 100% 命中。SemiStable 层部分命中（取决于子 Agent 的工具集和 profile 是否改变）。

### 4.4 安全性分析

- `LayerCache` 内部是 `Arc<Mutex<LayerCache>>`，线程安全
- 父子不会同时写入同一个 cache entry 的同一个 key（fingerprint 不同 → 不同 key）
- 缓存是只读性质的（渲染结果的复用），不存在一致性问题

---

## 五、改动 3：上下文改进 — 结构化继承 + ReactivationPrompt 优化

### 5.1 现状：上下文继承

`resolve_context_snapshot()` 输出一个 `ResolvedContextSnapshot`，其中 `composed_task` 是纯文本：

```
# Task
investigate the auth module

# Context
focus on regressions

# Parent Compact Summary
The user asked about authentication... (long text)

# Recent Tail
- user: show me the auth code
- assistant: I found the auth module at...
- tool[call-1]: file contents...
```

这个文本作为子 Agent `AgentState.messages` 的唯一一条 User 消息。

**问题：**
- 子 Agent 无法区分"我的任务"和"继承的背景"
- `single_line` 将每条 tail 消息截断到 200 字符
- 作为单条 User 消息没有 cache 断点
- 父的 structural context（workset、memory blocks）不传递

### 5.2 设计：分层注入上下文

将上下文从"拼到 User 消息"改为"注入到 PromptPlan 的不同位置"：

```
PromptPlan.system_blocks:
  ┌─ Stable 层 ─────────────────────────────┐
  │  Identity                                │ ← 共享 cache，命中
  │  Environment                             │ ← 共享 cache，命中
  └──────────────────────────────────────────┘
  ┌─ SemiStable 层 ──────────────────────────┐
  │  AGENTS.md                               │ ← 共享 cache，命中
  │  CapabilityPrompt                        │ ← 可能命中（工具子集不同）
  │  AgentProfileSummary                     │ ← 可能命中
  │  SkillSummary                            │ ← 可能命中
  └──────────────────────────────────────────┘
  ┌─ Inherited 层（新增）─────────────────────┐  ← 新增层，在 Dynamic 前
  │  Parent Compact Summary                  │
  │  Parent Recent Tail                      │
  └──────────────────────────────────────────┘
  ┌─ Dynamic 层 ─────────────────────────────┐
  │  WorkflowExamples                        │
  └──────────────────────────────────────────┘

PromptPlan.prepend_messages: (空)

AgentState.messages:
  [0] User: "investigate the auth module"    ← 只有任务本身

PromptPlan.append_messages: (空)
```

#### 实现方式

**步骤 5.2.1：新增 PromptLayer 变体**

```rust
// 文件：crates/runtime-prompt/src/block.rs
pub enum PromptLayer {
    Stable,
    SemiStable,
    Inherited,  // ← 新增
    Dynamic,
    #[default]
    Unspecified,
}
```

**步骤 5.2.2：在 PromptContext 中传递继承上下文**

`PromptContributor` 的 `contribute()` 只接受 `&PromptContext`，所以继承上下文需要通过它传递：

```rust
// 文件：crates/runtime-prompt/src/context.rs

pub struct PromptContext {
    // ... 现有字段 ...
    pub inherited_context: Option<InheritedContext>,  // ← 新增
}

pub struct InheritedContext {
    pub compact_summary: Option<String>,
    pub recent_tail: Vec<String>,
}
```

**步骤 5.2.3：新增 contributor**

```rust
// 文件：crates/runtime-prompt/src/contributors/
pub struct ParentContextContributor;

impl PromptContributor for ParentContextContributor {

impl PromptContributor for ParentContextContributor {
    fn contributor_id(&self) -> &str { "parent_context" }
    fn cache_version(&self) -> u32 { 1 }
    fn cache_fingerprint(&self, ctx: &PromptContext) -> String {
        // 从 PromptContext 读取继承上下文生成指纹
        match &ctx.inherited_context {
            Some(inherited) => format!("{}/{}",
                inherited.compact_summary.as_deref().unwrap_or("").len(),
                inherited.recent_tail.join("|").len()
            ),
            None => "none".to_string(),
        }
    }
    fn contribute(&self, ctx: &PromptContext) -> PromptContribution {
        let inherited = match &ctx.inherited_context {
            Some(i) => i,
            None => return PromptContribution::default(),
        };
        let mut blocks = Vec::new();
        if let Some(summary) = &inherited.compact_summary {
            blocks.push(BlockSpec::system_text(
                "parent-compact-summary",
                BlockKind::ProjectRules,
                "Parent Compact Summary",
                summary.clone(),
            ));
        }
        if !inherited.recent_tail.is_empty() {
            blocks.push(BlockSpec::system_text(
                "parent-recent-tail",
                BlockKind::ProjectRules,
                "Recent Parent Activity",
                inherited.recent_tail.join("\n"),
            ));
        }
        PromptContribution::new(blocks)
    }
}
```

**步骤 5.2.4：LayeredPromptBuilder 添加 Inherited 层**

```rust
// 文件：crates/runtime-prompt/src/layered_builder.rs

pub fn with_inherited_layer(mut self, contributors: Vec<Arc<dyn PromptContributor>>) -> Self {
    self.inherited_contributors = contributors;
    self
}
```

构建时增加 Inherited 层的渲染，插入在 SemiStable 和 Dynamic 之间：

```rust
for (layer_type, contributors) in [
    (LayerType::Stable, &self.stable_contributors),
    (LayerType::SemiStable, &self.semi_stable_contributors),
    (LayerType::Inherited, &self.inherited_contributors),  // ← 新增
    (LayerType::Dynamic, &self.dynamic_contributors),
] { ... }
```

**步骤 5.2.5：PromptRuntime.build_plan() 传递继承上下文**

在 `build_plan()` 中将 `ResolvedContextSnapshot` 的 `inherited_compact_summary` 和 `inherited_recent_tail` 注入 `PromptContext.inherited_context`。不需要新方法，只需扩展现有 `build_plan()` 接受可选的继承参数。

**步骤 5.2.6：修改 resolve_context_snapshot**

分离 `composed_task`（只含 Task + Context）和继承数据（由 PromptRuntime 注入 Inherited 层）：

```rust
// 文件：crates/runtime-execution/src/context.rs

pub fn resolve_context_snapshot(...) -> ResolvedContextSnapshot {
    // composed_task 不再包含 summary/tail
    let composed_task = format!("# Task\n{}", params.prompt.trim());
    // context 如果有则追加
    // ...

    // 继承数据分离输出
    let inherited_compact_summary = if overrides.include_compact_summary {
        parent_state.and_then(latest_compact_summary)
    } else { None };

    let inherited_recent_tail = if overrides.include_recent_tail {
        parent_state.map(|s| inherited_recent_tail_lines(s, overrides)).unwrap_or_default()
    } else { Vec::new() };

    ResolvedContextSnapshot { composed_task, inherited_compact_summary, inherited_recent_tail }
}
```

**步骤 5.2.7：修改 build_child_agent_state**

```rust
// 文件：crates/runtime-execution/src/prep.rs

pub fn build_child_agent_state(
    session_id: &str,
    working_dir: PathBuf,
    task: &str,  // 只有 Task + Context，不含 summary/tail
) -> AgentState {
    AgentState {
        messages: vec![LlmMessage::User {
            content: task.to_string(),
            origin: UserMessageOrigin::User,
        }],
        // ...
    }
}
```

### 5.3 缓存收益

| 改动前 | 改动后 | 收益 |
|--------|--------|------|
| summary + tail 拼到 User 消息 | 作为 Inherited 层 system block | Inherited 层可获得 cache_boundary |
| 每次子 Agent 调用 summary 都变 | summary 变化时 fingerprint 才变 | 相同父状态下多次 spawn 子 Agent 可命中 |
| User 消息内容不固定 | User 消息只有 Task + Context | 更容易在 step 间缓存 |

### 5.4 ReactivationPrompt 优化

#### 现状

ReactivationPrompt 作为 `UserMessage { origin: ReactivationPrompt }` 进入父会话的消息流，破坏消息缓存尾部。

#### 设计：拆分 ReactivationPrompt 为"触发消息 + Dynamic 层详情"

**核心约束：** 父会话需要被"唤醒"才能开始下一个 turn。当前 ReactivationPrompt 同时承担两个职责：(1) 触发新 turn (2) 传递交付详情。优化方案是将这两个职责分离。

**触发消息：** 保留一条简短、固定的 User 消息来触发 turn。因为内容固定，不会破坏缓存：

```rust
// 固定的触发文本，不携带交付细节
const REACTIVATION_TRIGGER: &str = "continue";

// reactivate_parent_agent_if_idle() 中
submit_prompt_with_origin(
    &parent_session_id,
    REACTIVATION_TRIGGER.to_string(),  // ← 固定文本，每次都一样
    UserMessageOrigin::ReactivationPrompt,
).await
```

**交付详情：** 通过 `PromptDeclaration` 注入 Dynamic 层，不进入消息流：

```rust
// 文件：crates/runtime-agent-loop/src/subagent.rs

pub fn build_parent_reactivation_blocks(
    notification: &ChildSessionNotification,
) -> Vec<BlockSpec> {
    vec![
        BlockSpec::system_text(
            "child-delivery",
            BlockKind::Skill,
            "Child Session Delivery",
            format_delivery_text(notification),
        )
    ]
}
```

**注入机制：**

需要在 `AgentLoop` 上增加一个"一次性 declaration"队列——注入后在下一个 turn 的 `build_plan()` 中消费并清除：

```rust
// 文件：crates/runtime-agent-loop/src/agent_loop.rs

impl AgentLoop {
    pub fn inject_ephemeral_declarations(&self, decls: Vec<PromptDeclaration>) {
        // 原子地追加到一次性队列
    }
}
```

`prompt_runtime.build_plan()` 在构建时读取并合并这些 declaration，用完即弃。

**收益：**
- 触发消息固定（`"continue"`），不破坏消息缓存尾部的内容多样性
- 交付详情在 Dynamic 层，不进入消息流
- 父会话的消息流保持连续性
- 多次 reactivation 的触发消息完全相同，对缓存友好

**风险与缓解：**
- `PromptDeclaration` 当前是静态的，需要扩展一次性注入机制——复杂度可控
- EventTranslator 已过滤 ReactivationPrompt 不回放为前端 UserMessage，保留此行为
- 需确保 `"continue"` 文本不会在 compaction 时被误认为是真正的用户输入

---

## 六、改动汇总与优先级

| 优先级 | 改动 | 影响范围 | 风险 | 收益 |
|--------|------|---------|------|------|
| **P0** | 存储分离（IndependentSession 默认） | `core/agent`, `runtime-execution/policy`, `core/projection` | 低 | 边界清晰、并发安全、调试容易 |
| **P0** | 共享 LayerCache | `runtime-prompt/layered_builder`, `prompt_runtime`, `loop_factory` | 低 | 子 Agent Stable 层零渲染开销 |
| **P1** | 结构化上下文继承 | `runtime-execution/context`, `runtime-prompt/block`, `prompt_runtime` | 中 | 上下文不丢失、有 cache 断点 |
| **P2** | ReactivationPrompt 移入动态层 | `subagent.rs`, `execution/mod.rs`, `prompt_runtime` | 中 | 保护消息缓存连续性 |

### 推荐实施顺序

```
Phase 1（P0，低风险高收益）：
  ├─ 3.2 翻转默认存储模式
  ├─ 3.2 简化投影过滤逻辑
  └─ 4.2 共享 LayerCache

Phase 2（P1，中风险中收益）：
  ├─ 5.2 新增 Inherited 层
  ├─ 5.2 修改 resolve_context_snapshot
  └─ 5.2 修改 build_child_agent_state

Phase 3（P2，中风险中收益）：
  └─ 5.4 ReactivationPrompt 移入动态层
```

---

## 七、验证标准

### Phase 1 验证

- `cargo test --workspace` 全部通过
- 手动测试：spawn 子 Agent → 检查子 session JSONL 是否独立写入
- 手动测试：查看 `PromptMetrics` 日志确认子 Agent Step 0 的 `cache_creation_input_tokens` 显著减少
- 手动测试：同时 spawn 多个子 Agent → 检查无写入冲突

### Phase 2 验证

- 子 Agent 的 system prompt 中出现 `Parent Compact Summary` 和 `Recent Parent Activity` 块
- 子 Agent 的 User 消息只包含 `# Task` + `# Context`
- `PromptPlan` 的 `Inherited` 层获得正确的 `cache_boundary`
- 测试：`single_line` 截断不再影响 tail 内容（完整文本注入）

### Phase 3 验证

- 父会话被 reactivation 后，消息序列中不出现 `ReactivationPrompt` 的 User 消息
- 交付信息出现在 system prompt 的 Dynamic 层
- 父会话的 `enable_message_caching(depth=3)` 缓存不被 reactivation 破坏

---

## 八、关键文件索引

### 存储分离

| 文件 | 改动 |
|------|------|
| `crates/core/src/agent/mod.rs:287` | 默认 `storage_mode` → `IndependentSession` |
| `crates/runtime-execution/src/policy.rs:66-74` | 移除实验性 flag 守卫 |
| `crates/core/src/projection/agent_state.rs:247-261` | 简化投影过滤 |
| `crates/storage/src/session/paths.rs` | 目录结构（无需改动，已支持） |

### 缓存共享

| 文件 | 改动 |
|------|------|
| `crates/runtime-prompt/src/layered_builder.rs` | `with_shared_cache()` 方法 |
| `crates/runtime-agent-loop/src/prompt_runtime.rs` | `layer_cache()` 暴露 |
| `crates/runtime-agent-loop/src/agent_loop.rs` | `prompt_layer_cache()` 暴露 |
| `crates/runtime/src/service/loop_factory.rs` | `parent_layer_cache` 参数注入 |

### 上下文改进

| 文件 | 改动 |
|------|------|
| `crates/runtime-prompt/src/block.rs` | 新增 `Inherited` 层 |
| `crates/runtime-prompt/src/contributors/` | 新增 `ParentContextContributor` |
| `crates/runtime-execution/src/context.rs` | `resolve_context_snapshot()` 分离输出 |
| `crates/runtime-execution/src/prep.rs` | `build_child_agent_state()` 简化 |
| `crates/runtime-agent-loop/src/subagent.rs` | ReactivationPrompt → system block |
| `crates/runtime/src/service/execution/mod.rs` | reactivation 流程改用 declaration 注入 |
