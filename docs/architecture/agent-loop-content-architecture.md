# AgentLoop 内容架构重构计划

> 最后更新：2026-04-03（整合另一 agent 建议，补充 Phase 0 / Stage 分类 / 约束强化）
> 状态：Proposed
> 影响范围：`crates/runtime-agent-loop`、`crates/runtime-prompt`、`crates/runtime`

本文档定义 AgentLoop 下一轮重构的目标边界与落地顺序。重点不是再拆 workspace crate，而是把当前 `turn_runner` 中混杂的 Prompt、Context、Compaction 与执行状态机职责切开。

## 结论

本轮采用以下总设计：

- 宏观结构：`Ports & Adapters + Event Sourcing`
- Prompt：`Contributor + Builder`
- Context：`Pipeline`
- Compaction：`Policy + Strategy + Rebuilder`
- Policy：保留 `Verdict` 模型，接口继续异步；`CompactionPolicy` 只是局部信号，不替代全局 `PolicyEngine`
- AgentLoop：保持 sealed 的 `Application Service + State Machine`

本轮**不新增 crate**。现有 `runtime-agent-loop` / `runtime-prompt` / `runtime-skill-loader` 的 crate 边界已经足够，下一步优先做 crate 内部的职责重组。

## 为什么现在要重构

当前的主要问题不在 `runtime-prompt`，而在 `runtime-agent-loop` 的执行层：

- `AgentLoop` 同时持有 prompt 组装、skill surface、自动 compact 参数、tool noise 参数，字段层面已经泄漏多类策略。
- `turn_runner` 同时负责：
  - 构建 prompt context
  - 拼接 request messages
  - 执行 microcompact / auto compact / reactive compact
  - 计算 token metrics
  - 调用 policy
  - 处理 LLM/tool 循环
- `PromptComposer` 的输出当前被直接和历史消息拼装，导致 Prompt 与 Context 的边界不清楚。
- compact 现在既承担“是否触发”，又承担“如何压缩”，还承担“压缩后如何恢复 active context”，这些变化点被耦合在一起。

结果是：

- 想替换 compact 策略时，需要碰执行主循环。
- 想开放上下文选材能力时，需要改 `turn_runner`。
- 想优化 prompt 结构时，容易误伤消息选材与请求装配。

## 这轮不做什么

- 不再继续拆 `runtime-context` / `runtime-compaction` 之类的新 crate。
- 不重写 `PromptComposer` 为另一套全新命名体系。
- 不把 `PolicyEngine` 改成同步接口。
- 不在本轮引入完整的 registry/overlay 配置系统。
- 不在 Phase 1 引入新的 prompt 输入对象；现有 `PromptContext` 继续作为 PromptRuntime 的缓冲层。
- 不在本轮重写 reactive compact 为多策略实现；当前基于消息顺序递进丢弃的逻辑保留到 Phase 3 再演进。

这些方向都可能成立，但现在继续拆只会过早固化接口。

## 目标边界

重构后，主链路应收敛为：

```text
SessionState / EventLog
  -> ContextPipeline
  -> ContextBundle

PromptRuntime
  -> PromptPlan

ContextBundle
  -> maybe compact / rebuild conversation view

ContextBundle + PromptPlan + ToolDefinitions
  -> RequestAssembler
  -> ModelRequest

ModelRequest
  -> PolicyEngine
  -> LLM Call
  -> Tool Cycle
```

其中：

- Prompt 只负责“说明书长什么样”
- Context 只负责“当前给模型哪些材料”
- Compaction 只负责“历史如何折叠，以及折叠后如何重建 conversation view”
- AgentLoop 只负责“turn 状态机和执行语义”

## 运行时对象

### PromptRuntime

`PromptRuntime` 负责桥接 `runtime-prompt`，但不吞掉历史消息选材。

建议职责：

- 持有 `PromptComposer`
- 持有 prompt 可见的 capability descriptors
- 持有 `PromptDeclaration`
- 持有 `SkillCatalog`
- 根据 loop 提供的输入快照生成 `PromptContext`
- 产出 `PromptPlan`

注意：

- 优先**演进现有 `PromptPlan`**，不要为了概念漂亮立即新造 `PromptScaffold`。
- `PromptRuntime` 读取的是 loop 提供的输入快照，不直接读取或修改 session 真相（event log、storage、session state）。
- `PromptRuntime` 不负责：
  - recent tail 选择
  - tool result trim
  - compact 视图恢复
  - token budget 裁剪

### ContextRuntime

`ContextRuntime` 只负责上下文材料选择，不碰 prompt 语义，也不直接决定最终 request 格式。

建议结构：

```rust
pub struct ContextRuntime {
    stages: Vec<Box<dyn ContextStage>>,
}

pub trait ContextStage: Send + Sync {
    fn apply(
        &self,
        bundle: ContextBundle,
        ctx: &ContextStageContext,
    ) -> anyhow::Result<ContextBundle>;
}
```

建议 `ContextBundle` 至少包含：

```rust
pub struct ConversationView {
    /// 当前对模型可见的会话视图。
    /// 不等同于完整历史，也不等同于 event log replay 结果；
    /// 它是经过选材、裁剪、compact 恢复后的"模型视角"消息序列。
    pub messages: Vec<LlmMessage>,
}

pub struct ContextBundle {
    pub conversation: ConversationView,
    pub workset: Vec<ContextBlock>,
    pub memory: Vec<ContextBlock>,
    pub diagnostics: Vec<ContextDiagnostic>,
    pub budget_state: TokenBudgetState,
}
```

建议首批 stage（按职责分类）：

- **Materialize（补材料）**：`BaselineStage` / `RecentTailStage` / `WorksetStage`
- **Transform（变换）**：`CompactionViewStage`
- **Prune（裁剪）**：`ToolNoiseTrimStage` / `BudgetTrimStage`

分类目的：防止 stage 长成"什么都能干"的 God Object；每个 stage 应只属于一类职责。

> `ToolNoiseTrimStage` 现在负责 conversation 级 microcompact；`RequestAssembler` 只做最终请求编码与快照，
> 不再承担工具结果裁剪职责。

约束：

- `ContextRuntime` 不能再次长成第二个 God Object。
- 它不应该知道 prompt contributors、tool routing 或 policy verdict 细节。
- `ContextStage` 只做纯变换：`Pipeline` 做变换，`Runtime` 准备材料，`Loop` 决定何时调用。
- `ContextStage` 不直接触发 compact，不直接调用 policy，不直接做 request 装配。
- `ContextStageContext` 只提供只读快照，不提供会写状态或触发副作用的能力。stage 不得：写 event、发审批、查 provider 远端状态、调工具、触发 compact。

### CompactionRuntime

`CompactionRuntime` 单独承载 compact 的三个变化点：

- 何时触发
- 如何压缩
- 压完如何重建 conversation view

建议结构：

```rust
pub struct CompactionRuntime {
    pub policy: Arc<dyn CompactionPolicy>,
    pub strategy: Arc<dyn CompactionStrategy>,
    pub rebuilder: Arc<dyn CompactionRebuilder>,
}
```

对应接口：

```rust
pub trait CompactionPolicy: Send + Sync {
    fn should_compact(
        &self,
        snapshot: &PromptTokenSnapshot,
    ) -> Option<CompactionReason>;
}

#[async_trait]
pub trait CompactionStrategy: Send + Sync {
    async fn compact(
        &self,
        input: CompactionInput,
    ) -> anyhow::Result<Option<CompactionArtifact>>;
}

pub trait CompactionRebuilder: Send + Sync {
    fn rebuild(
        &self,
        artifact: &CompactionArtifact,
        tail: &[StoredEvent],
    ) -> anyhow::Result<ConversationView>;
}
```

`CompactionArtifact` 不应只保存摘要字符串，至少需要：

```rust
pub struct CompactionArtifact {
    pub summary: String,
    pub source_range: EventRange,
    pub preserved_tail_start: u64,
    pub strategy_id: String,
    pub pre_tokens: usize,
    pub post_tokens_estimate: usize,
    /// 压缩完成时的 storage_seq，便于 session rebuild 与调试定位。
    pub compacted_at_seq: u64,
    /// 触发本次压缩的原因（auto / reactive / manual），便于策略分析与排查。
    pub trigger: CompactionReason,
}
```

这样后续的 session rebuild、诊断、可视化和策略切换才有足够上下文。

额外约束：

- `CompactionRebuilder` 只重建窄化后的 `ConversationView`，不返回完整 `ContextBundle`。
- 这样 compact 不会顺势长成第二个上下文编排器，也不会吞掉 `workset` / `memory` / `budget_state` 的职责。
- manual compact 也走同一条 `CompactionRuntime` 管线；`runtime` 服务层只负责提供真实
  `StoredEvent` tail snapshot，而不是再维护一条基于投影消息的独立 rebuild 路径。

### RequestAssembler

这是这轮新增的薄边界，用来避免 PromptRuntime 直接吞掉 Context。

职责：

- 把 `PromptPlan`
- `ContextBundle`
- tool definitions
- system prompt

组装成最终 `ModelRequest`

建议接口：

```rust
pub struct RequestAssembler;

impl RequestAssembler {
    pub fn assemble(
        &self,
        prompt: &PromptPlan,
        context: ContextBundle,
        tools: Vec<ToolDefinition>,
    ) -> anyhow::Result<ModelRequest>;
}
```

约束：

- RequestAssembler 不做策略判断
- RequestAssembler 不做 compact 决策
- RequestAssembler 不做 prompt 渲染

它只是最后一道“请求序列化边界”

## AgentLoop 的目标形态

重构后，`turn_runner` 应只保留状态机骨架：

```text
build prompt plan
-> build context bundle
-> maybe compact / rebuild
-> assemble request
-> policy check
-> call llm
-> tool cycle
-> continue or stop
```

这意味着：

- LLM/tool 循环仍然属于 `AgentLoop`
- prompt、context、compaction 变成可替换的内部协作者
- 用户以后真正自定义的是策略对象，而不是 loop 本身

`TurnOutcome` 至少包含：`Completed`（正常结束）、`RequiresUserInput`（等待用户）、`Cancelled`（用户取消）、`Error`（不可恢复错误）。状态转换由 `AgentLoop` 主干统一处理，不分散在 stage 或 runtime 内部。

## 与现有 crate 的职责关系

### `crates/runtime-prompt`

继续负责：

- `PromptComposer`
- contributor/block/render/diagnostics/declarations
- `PromptPlan`

不再负责：

- 历史消息选择
- compact 触发与恢复
- token budget trim

### `crates/runtime-agent-loop`

继续负责：

- `AgentLoop`
- `turn_runner`
- `tool_cycle`
- `llm_cycle`

新增内部模块，优先建议：

- `prompt_runtime.rs`
- `context_pipeline.rs`
- `compaction_runtime.rs`
- `request_assembler.rs`

### `crates/runtime`

继续作为门面：

- `RuntimeService`
- session CRUD
- runtime surface 装配
- provider/config 适配

不应重新吸收 AgentLoop 内部的 prompt/context/compaction 细节。

## Policy 的边界

`PolicyEngine` 保持当前 verdict 模型，但接口继续异步：

- 可能需要用户确认
- 可能需要外部 approval broker
- 可能需要运行时状态查询

因此本轮只做职责收口，不改变异步边界。

同时明确 `CompactionPolicy -> PolicyEngine` 的两层决策链：

1. `CompactionPolicy` 只根据 token pressure、上下文形态和策略参数给出“建议 compact”的局部信号。
2. `AgentLoop` 收到该信号后，仍需通过 `PolicyEngine` 做全局决策（当前 `PolicyEngine` 尚未暴露 `decide_context_strategy` 或等效钩子，Phase 1 收口运行时时需新增）。
3. 只有当全局决策允许 `Compact` 时，`CompactionStrategy` 和 `CompactionRebuilder` 才会真正执行。

这条链路的目的，是避免 `CompactionPolicy` 与 `PolicyEngine` 同时拥有最终裁决权，形成两个 policy source 打架。

同时明确：

- Policy 不嵌进 `ContextRuntime`
- Policy 不嵌进 `CompactionStrategy`
- `CompactionPolicy` 不等于 `PolicyEngine`
- Policy 仍由 `AgentLoop` 在 request / capability call / context strategy 边界统一调用

## 为什么现在不拆新 crate

当前最值得重构的是接口与职责，不是编译边界。

如果现在把 `context` / `compaction` 直接拆成新 crate，会产生两个问题：

1. 过早把不稳定接口变成 crate API
2. 让大量仍在演化的内部实现被迫走跨 crate 依赖和兼容约束

因此，这轮遵循：

- 先在 `runtime-agent-loop` 内部切模块
- 先把抽象跑顺
- 之后再看 `ContextRuntime` 或 `CompactionRuntime` 是否值得独立 crate

## 第二阶段的预留口

虽然本轮不做完整的 `Registry + Overlay`，但接口形状要为未来留口。

同样地，Prompt 侧这轮继续沿用现有 `PromptContext` 作为缓冲层；如果未来确实需要进一步把 Prompt 输入对象瘦身，再作为后续阶段单独处理，而不是塞进 Phase 1。

例如：

```rust
pub struct ContextRuntime {
    stages: Vec<Box<dyn ContextStage>>,
}
```

将来可以平滑替换为：

- registry lookup
- config overlay
- 插件注入 stage

而不需要重写 `AgentLoop` 主干。

同理，`CompactionRuntime` 也应通过 trait 对外表达，而不是把策略逻辑写回 `turn_runner`。

## 落地顺序

### Phase 0：行为基线锁定

在正式重构前，先补一组"当前行为快照测试"，锁定现有 `turn_runner` 的语义，否则后续重构无法判断是否引入回退。

至少覆盖以下场景：

- 普通单轮无工具
- tool call 后继续一轮
- `max_tokens` 自动续命（auto-continue nudge）
- auto compact
- reactive compact
- policy deny / ask
- cancel / interrupted

验收：

- 以上场景均有可运行的测试或 fixture，能在重构前后对比行为一致性
- 优先复用现有 `ScriptedProvider` / `FailingProvider` / `RecordingProvider` 等测试工具，在现有 `tests.rs` 基础上补齐缺失场景，而非另建测试框架

### Phase 1：收口运行时字段

目标：

- 在 `AgentLoop` 中收出：
  - `PromptRuntime`
  - `ContextRuntime`
  - `CompactionRuntime`
  - `RequestAssembler`

- 沿用现有 `PromptContext` 作为 PromptRuntime 的输入缓冲，不在这一阶段引入新的 Prompt 输入抽象。

字段归属明确：

- `auto_compact_enabled` / `compact_threshold_percent` / `compact_keep_recent_turns` → 归入 `CompactionRuntime` 配置
- `tool_result_max_bytes` → 归入 `ContextRuntime`（属于 tool noise trim）
- `max_tool_concurrency` → 保留在 `AgentLoop`（属于执行调度，不属于内容管理）
- `PolicyEngine` 需新增 `decide_context_strategy` 或等效钩子，否则 `CompactionPolicy` 的建议无法经过全局策略裁决

验收：

- `AgentLoop` 顶层字段不再直接暴露过多 prompt/compact 配置细节
- `turn_runner` 可以通过运行时对象访问协作者，而不是直接摸所有字段

### Phase 2：接入 RequestAssembler 骨架 + 抽出 ContextPipeline

目标：

- 先立住 `RequestAssembler` 作为唯一请求装配边界，防止 `ContextBundle` 和 `PromptPlan` 继续偷偷耦合
- 把消息选材、tool noise trim、budget trim、compact view 应用，从 `turn_runner` 搬入 `ContextRuntime`

验收：

- `turn_runner` 不再直接手工拼接完整 request messages
- `ContextBundle` 成为显式中间结果
- `RequestAssembler` 成为 `PromptPlan + ContextBundle + Tools -> ModelRequest` 的唯一入口

### Phase 3：抽出 Compaction 三件套

目标：

- 让 `auto_compact` / reactive compact 从“直接改消息数组”，演进为“产出 artifact，再 rebuild context”

验收：

- `CompactionArtifact` 显式存在
- reactive compact 不再依赖散落的 request 重建逻辑

### Phase 4：清理命名与遗留逻辑

目标：

- 优先演进现有类型，而不是平行创造新术语
- 收敛掉 `turn_runner` 中剩余的 glue logic

验收：

- 核心骨架清晰
- 对外概念数量减少而不是增加

## 测试策略

### `runtime-agent-loop`

- 为 `ContextStage` 新增单元测试，验证 stage 顺序和裁剪行为
- 为 `CompactionPolicy` / `Strategy` / `Rebuilder` 新增单元测试
- 为 `RequestAssembler` 新增 request 顺序契约测试
- 为 `turn_runner` 保留集成测试，验证状态机主流程不回退

### `runtime-prompt`

- 保留现有 contributor / `PromptComposer` 测试
- 补充 `PromptPlan` 语义收窄后的回归测试

### `runtime`

- 保留 `RuntimeService` 与 bootstrap 测试
- 确保 facade 层不重新吸入 loop 内部实现细节

## 验收标准

- `AgentLoop` 只负责编排执行语义，不再同时承担 prompt/context/compaction 细节
- `PromptRuntime` 不碰历史选材
- `ContextRuntime` 不碰 prompt 语义
- `ContextStage` 保持纯变换，不拥有调用时机决策权
- `CompactionRuntime` 不再只是 `if/else + summary string`
- `CompactionRebuilder` 只返回 `ConversationView`
- `CompactionPolicy` 服从 `PolicyEngine` 的全局 context strategy 决策
- `RequestAssembler` 成为唯一请求装配边界
- 不新增 crate，也不留下新的平行抽象体系

## 一句话原则

Prompt 负责说明书，Context 负责材料选择，Compact 负责历史折叠与会话视图恢复，Assembler 负责请求编码，Loop 负责执行语义。
