# ADR-0008: AgentLoop 内容架构——Prompt/Context/Compaction/Assembler 四层分离

**状态**: 已实施
**日期**: 2026-04-03
**实施提交**: b157932

---

## 背景

重构前的 `runtime-agent-loop` 存在严重的职责混淆：

1. **`AgentLoop` 字段泄漏**：同时持有 prompt 组装、skill surface、自动 compact 参数、tool noise 参数等多类策略配置

2. **`turn_runner` 职责过载**：同时负责
   - 构建 prompt context
   - 拼接 request messages
   - 执行 microcompact / auto compact / reactive compact
   - 计算 token metrics
   - 调用 policy
   - 处理 LLM/tool 循环

3. **边界模糊**：
   - `PromptComposer` 输出被直接和历史消息拼装，Prompt 与 Context 边界不清
   - compact 同时承担"是否触发"、"如何压缩"、"压缩后如何恢复 active context"三个变化点

**结果**：想替换 compact 策略需要碰执行主循环；想开放上下文选材需要改 `turn_runner`；想优化 prompt 结构容易误伤消息选材。

---

## 决策

### 总体架构：Ports & Adapters + Event Sourcing

不新增 crate，在现有 `runtime-agent-loop` 内部切分四个独立运行时：

```
SessionState / EventLog
    → ContextRuntime（材料选择）
    → ContextBundle

PromptRuntime（说明书构建）
    → PromptPlan

ContextBundle + PromptPlan + ToolDefinitions
    → RequestAssembler（请求编码）
    → ModelRequest

CompactionRuntime（历史折叠）
    → CompactionArtifact
    → ConversationView

ModelRequest
    → PolicyEngine
    → LLM Call
    → Tool Cycle
```

### 1. PromptRuntime：只负责"说明书长什么样"

**文件**: `crates/runtime-agent-loop/src/prompt_runtime.rs`

```rust
pub(crate) struct PromptRuntime {
    composer: PromptComposer,
    tool_names: Vec<String>,
    capability_descriptors: Vec<CapabilityDescriptor>,
    prompt_declarations: Vec<PromptDeclaration>,
    skill_catalog: Arc<SkillCatalog>,
}
```

**约束**：
- 不碰历史消息选材
- 不碰 compact 触发与恢复
- 不碰 token budget trim
- 只读取 loop 提供的输入快照，不直接读取或修改 session 真相

### 2. ContextRuntime：只负责"当前给模型哪些材料"

**文件**: `crates/runtime-agent-loop/src/context_pipeline.rs`

```rust
pub(crate) struct ContextRuntime {
    stages: Vec<Box<dyn ContextStage>>,
}

pub(crate) trait ContextStage: Send + Sync {
    fn apply(&self, bundle: ContextBundle, ctx: &ContextStageContext) -> Result<ContextBundle>;
}
```

**Stage 职责分类**（防止 God Object）：

| 分类 | Stage | 职责 |
|------|-------|------|
| Materialize | `BaselineStage` / `RecentTailStage` / `WorksetStage` | 补材料 |
| Transform | `CompactionViewStage` | 变换 |
| Prune | `ToolNoiseTrimStage` / `BudgetTrimStage` | 裁剪 |

**约束**：
- 不知道 prompt contributors、tool routing 或 policy verdict 细节
- Stage 只做纯变换，不触发 compact、不调 policy、不做 request 装配
- `ContextStageContext` 只提供只读快照，stage 不得写 event、发审批、调工具

### 3. CompactionRuntime：统一承载三个变化点

**文件**: `crates/runtime-agent-loop/src/compaction_runtime.rs`

```rust
pub(crate) struct CompactionRuntime {
    enabled: bool,
    keep_recent_turns: usize,
    threshold_percent: u8,
    policy: Arc<dyn CompactionPolicy>,      // 何时触发
    strategy: Arc<dyn CompactionStrategy>,  // 如何压缩
    rebuilder: Arc<dyn CompactionRebuilder>, // 压完如何重建
}
```

**CompactionArtifact** 不只保存摘要字符串：

```rust
pub(crate) struct CompactionArtifact {
    pub summary: String,
    pub source_range: EventRange,
    pub preserved_tail_start: u64,
    pub strategy_id: String,
    pub pre_tokens: usize,
    pub post_tokens_estimate: usize,
    pub compacted_at_seq: u64,    // 便于 session rebuild 与调试
    pub trigger: CompactionReason, // auto / reactive / manual
}
```

**约束**：
- `CompactionRebuilder` 只返回 `ConversationView`，不返回完整 `ContextBundle`
- manual compact 也走同一条管线
- `CompactionPolicy` 服从 `PolicyEngine` 的全局决策（两层决策链）

### 4. RequestAssembler：唯一请求装配边界

**文件**: `crates/runtime-agent-loop/src/request_assembler.rs`

```rust
pub(crate) struct RequestAssembler;

impl RequestAssembler {
    pub(crate) fn assemble(
        &self,
        prompt: &PromptPlan,
        context: ContextBundle,
        tools: Vec<ToolDefinition>,
    ) -> Result<ModelRequest>;
}
```

**约束**：
- 不做策略判断
- 不做 compact 决策
- 不做 prompt 渲染
- 只是最后一道"请求序列化边界"

### 5. Policy 两层决策链

**问题**：避免 `CompactionPolicy` 与 `PolicyEngine` 同时拥有最终裁决权。

**决策链**：

1. `CompactionPolicy.should_compact()` → 局部信号（建议 compact）
2. `CompactionRuntime.build_context_decision()` → 封装为 `ContextDecisionInput`
3. `PolicyEngine.decide_context_strategy()` → 全局裁决（新增钩子）
4. 只有当全局决策允许 `Compact` 时，`CompactionStrategy` 和 `CompactionRebuilder` 才执行

**新增接口** (`crates/core/src/policy/engine.rs`):

```rust
pub trait PolicyEngine: Send + Sync {
    // ... 已有方法 ...

    async fn decide_context_strategy(
        &self,
        input: &ContextDecisionInput,
        ctx: &PolicyContext,
    ) -> Result<ContextStrategy>;
}
```

### 6. turn_runner 收口为状态机骨架

重构后 `turn_runner` 只保留执行流程骨架：

```
build prompt plan
→ build context bundle
→ maybe compact / rebuild
→ assemble request
→ policy check
→ call llm
→ tool cycle
→ continue or stop
```

---

## 核心思想

### 一句话原则

**Prompt 负责说明书，Context 负责材料选择，Compact 负责历史折叠与会话视图恢复，Assembler 负责请求编码，Loop 负责执行语义。**

### 为什么不拆新 crate

当前最值得重构的是接口与职责，不是编译边界。

如果直接拆 `context` / `compaction` 为新 crate：
1. 过早把不稳定接口变成 crate API
2. 让大量仍在演化的内部实现被迫走跨 crate 依赖和兼容约束

因此：**先在 `runtime-agent-loop` 内部切模块，先把抽象跑顺，之后再决定是否独立 crate。**

### 为什么需要 CompactionArtifact

压缩不只是"生成摘要字符串"，后续需要：
- session rebuild：从 artifact 恢复会话视图
- 诊断排查：知道何时、为何、如何压缩
- 策略切换：不同策略的 artifact 格式可能不同
- 可视化：展示压缩前后对比

### ConversationView vs ContextBundle 的区别

- `ConversationView`：模型可见的消息序列（窄化）
- `ContextBundle`：完整上下文包（含 workset、memory、diagnostics、budget_state）

**`CompactionRebuilder` 只返回 `ConversationView`**，这样 compact 不会顺势长成第二个上下文编排器。

---

## 与其他 ADR 的关系

| ADR | 关系 |
|-----|------|
| ADR-0005 | Policy 决策平面分离，本 ADR 新增 `decide_context_strategy` 决策点 |
| ADR-0006 | Turn 状态机化，本 ADR 在 `turn_runner` 骨架上叠加内容分层 |
| ADR-0007 | 分层 Prompt 构建器，本 ADR 的 `PromptRuntime` 桥接该层 |

---

## 实施阶段

| Phase | 目标 | 状态 |
|-------|------|------|
| 0 | 行为基线锁定（回归测试矩阵） | ✅ |
| 1 | 收口运行时字段（四个 Runtime + Policy 钩子） | ✅ |
| 2 | RequestAssembler 骨架 + ContextPipeline | ✅ |
| 3 | Compaction 三件套（Artifact/Policy/Strategy/Rebuilder） | ✅ |
| 4 | 清理命名与遗留逻辑 | ✅ |

---

## Current Implementation Status

截至 2026-04-03，已全部落地：

| 组件 | 位置 |
|------|------|
| `PromptRuntime` | `crates/runtime-agent-loop/src/prompt_runtime.rs` |
| `ContextRuntime` | `crates/runtime-agent-loop/src/context_pipeline.rs` |
| `CompactionRuntime` | `crates/runtime-agent-loop/src/compaction_runtime.rs` |
| `RequestAssembler` | `crates/runtime-agent-loop/src/request_assembler.rs` |
| `PolicyEngine.decide_context_strategy` | `crates/core/src/policy/engine.rs:270` |
| `ContextDecisionInput` | `crates/core/src/policy/engine.rs:84-97` |
| `ContextStrategy` | `crates/core/src/policy/engine.rs:103-109` |
| `CompactionArtifact` | `crates/runtime-agent-loop/src/compaction_runtime.rs:52-65` |
| `ConversationView` | `crates/runtime-agent-loop/src/context_pipeline.rs:16-24` |
| `ContextBundle` | `crates/runtime-agent-loop/src/context_pipeline.rs:48-55` |
| Phase 0 回归测试 | `crates/runtime-agent-loop/src/agent_loop/tests/regression.rs` |
| 实施提交 | b157932 |
