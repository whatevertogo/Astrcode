## Context

AstrCode 当前的"模式"控制能力分散在多个组件中，缺少统一概念：

- `AgentProfile` (`core/src/agent/mod.rs`) 定义身份级 allowed_tools，但不随对话阶段变化
- `CapabilityRouter` (`kernel/src/registry/router.rs`) 有 `subset_for_tools()` 能力，但仅在子 agent 路径使用
- `PolicyEngine` (`core/src/policy/engine.rs`) 做运行时审批（Allow/Deny/Ask），但 LLM 在审批前已经看到了不该看到的工具
- `AgentPromptSubmission` (`session-runtime/src/turn/submit.rs`) 已经是 per-turn 包络雏形，携带 capability_router + prompt_declarations，但只在子 agent 场景充分使用
- `TurnExecutionResources` (`session-runtime/src/turn/runner.rs`) 在 turn 开始时固定 tools（第 157 行），整个 step loop 共享同一份

本设计在现有架构上引入 CollaborationMode 作为显式协作阶段，以 ModeSpec 为一等规格对象统一控制工具授予、提示词注入和转换规则。

## Goals / Non-Goals

**Goals:**

- 引入 `CollaborationMode` 枚举（Plan / Execute / Review）作为 session 级 durable truth
- 以 `ModeSpec` 声明式定义每个模式的工具授予、提示词、进入策略和转换规则
- 通过 `compile_mode_spec()` 在 turn 边界编译出 visible tools + prompt directives
- 提供 `switchMode` tool（LLM 可调用）+ `/mode` command（用户输入）+ UI 快捷键三个切换入口
- 引入 `ModeArtifact` 双层模型（Ref + Body）作为模式间结构化交接协议
- 类型设计预留 SDK 自定义 mode 的扩展点

**Non-Goals:**

- 不做 step 级工具切换
- 不做 SandboxProfile / ReasoningEffort / CapabilityBudget
- 不做 PluginModeCatalog（但类型预留）
- 不做隐式意图识别
- 不做 Phase 状态机（Explore → DraftPlan → ...），先只做三态切换

## Decisions

### Decision 1: 模式真相归属 session-runtime

**选择：** `SessionState.session_mode: CollaborationMode` 作为 durable truth

**理由：**
- 模式满足三个条件：会跨 turn 持续存在、会影响后续 turn 行为、需要被恢复/重放/审计
- SessionState 已经是 per-field `StdMutex` 模式，新增 `session_mode` 字段遵循现有风格
- 模式切换通过 `StorageEvent::ModeChanged` 持久化，与现有事件模型一致

**替代方案：**
- 放在 AgentProfile → 被否决：profile 是稳定身份，不是临时阶段
- 放在 PolicyEngine → 被否决：只能"拦住"不能"先收口"，LLM 仍能看到不该看的工具
- 放在 protocol/adapter → 被否决：传输层不该拥有模式真相

### Decision 2: 工具授予采用"只给"模式

**选择：** `ToolGrantRule` 枚举定义授予规则，compile 阶段白名单过滤

```rust
pub enum ToolGrantRule {
    Named(String),
    SideEffect(SideEffect),
    All,
}
```

**理由：**
- 白名单模式比黑名单更安全——LLM 天然不知道未授予的工具
- `SideEffect(None)` 可以一次性授予所有只读工具，不用逐个列名字
- 与 `CapabilitySpec.side_effect` 字段对齐，无需新概念
- 复用 `CapabilityRouter` 的 `subset_for_tools()` 能力

**替代方案：**
- 黑名单过滤 → 被否决：LLM 看到完整列表再被截断，产生"被剥夺"幻觉
- 硬编码工具名列表 → 被否决：MCP 工具动态注册，无法静态枚举

### Decision 3: Turn 级工具切换 + Step 级 prompt override

**选择：** 工具集在 turn 边界编译并固定；step 级仅通过 prompt override 影响行为指导

**理由：**
- `TurnExecutionResources.tools` 在 turn 创建时确定（`runner.rs:157`），改为可变需要大重构
- `assemble_prompt_request` 每 step 都调，prompt directives 天然支持 per-step 变化
- turn 级持久化更容易事件化、恢复、审计；step 级太容易把状态机炸复杂
- LLM 调用 `switchMode` 后返回"下一 turn 将使用新工具集"，当前 step 可继续（plan 的只读工具无副作用）

**替代方案：**
- step 级工具切换 → 延后：需要让 `TurnExecutionResources.tools` 可变或每 step 重新编译，代价大
- 只做 turn 级不做 step override → 被否决：step override 成本极低且给 LLM 即时反馈

### Decision 4: ModeArtifact 双层模型

**选择：** Ref（轻量引用，走事件/UI/审批）+ Body（完整负载，走 render_to_prompt）

```
ModeArtifactRef   → StorageEvent 持久化、UI 展示、compact summary
ModeArtifactBody  → Plan(PlanContent) / Review(ReviewContent) / Custom { schema_id, data }
```

**理由：**
- 复用 AstrCode 已有的 ArtifactRef + SubRunHandoff 模式
- Builtin body 用强类型 Rust struct 保证编译期安全
- Custom body 用 `{ schema_id, schema_version, data: Value }` 支持 SDK 扩展
- `ArtifactRenderer` trait 负责将 Body 渲染成 `PromptDeclaration`，与现有 prompt 管道对接
- 渲染分级（Summary/Compact/Full）应对不同 context 压力

**替代方案：**
- 纯 `serde_json::Value` → 被否决：消费方无类型信息，UI 和审批流失去稳定结构
- 纯 tagged union 不带 Custom → 被否决：不支持 SDK 自定义 mode
- schema 验证在 compile time → 不可行：自定义 mode 的 schema 在运行时注册

### Decision 5: 统一模式切换入口

**选择：** 所有触发源（tool / command / UI 快捷键）汇聚到 `apply_mode_transition()`

**理由：**
- 与 `submit_prompt_inner` 作为统一 submit 入口的模式一致
- 统一做转换合法性验证、entry_policy 检查、事件广播
- 三种触发源的差异仅在"如何到达这个函数"，核心逻辑不重复

**实现位置：** `session-runtime/src/turn/mode_transition.rs` 新模块

### Decision 6: ModeCatalog trait 分层

**选择：** core 定义 trait，application 提供 BuiltinModeCatalog，未来 PluginModeCatalog

```rust
// core
pub trait ModeCatalog: Send + Sync {
    fn list_modes(&self) -> Vec<ModeSpec>;
    fn resolve_mode(&self, id: &str) -> Option<ModeSpec>;
}

// application
pub struct BuiltinModeCatalog { /* plan/execute/review */ }

// TODO: 未来
// pub struct PluginModeCatalog { /* 从 SDK 插件加载 */ }
```

**理由：**
- 与 `AgentProfileCatalog` trait 的分层模式完全一致
- core 只定义接口和稳定类型，application 负责注册和生命周期
- 预留 SDK 扩展点但不在本次实现

### Decision 7: 新增核心类型归属 core

**新增文件：** `core/src/mode/mod.rs`

包含：CollaborationMode 枚举、ModeSpec、ToolGrantRule、ModeEntryPolicy、ModeTransition、ModeArtifactRef、ModeArtifactBody、PlanContent、ReviewContent、ArtifactStatus、ArtifactRenderer trait、ModeCatalog trait

**依赖方向：**
```
core (types + traits)
  ↑
session-runtime (truth + compile + transition)
  ↑
application (catalog registration + orchestration wiring)
```

不引入新的 crate 边界。类型量不大，放 core 符合"跨 crate 稳定成立的语义"原则。

## Risks / Trade-offs

**[Risk] Turn 级工具切换导致 LLM 在 plan mode 调用 switchMode("execute") 后仍需等一个 turn**
→ 缓解：switchMode tool 返回明确提示"模式已切换，下一 turn 将使用完整工具集"。当前 step 可继续产出方案文本。对大多数工作流来说，plan → execute 的 turn 边界切换是自然的。

**[Risk] ModeSpec 的 tool_grants 与动态 MCP 工具对齐**
→ 缓解：`ToolGrantRule::SideEffect(SideEffect::None)` 按类别授予而非按名称，MCP 工具只要 `side_effect == None` 就自动纳入 plan mode。但需要确保 MCP 工具的 side_effect 标注准确。

**[Risk] ModeArtifact Custom body 的 schema 验证**
→ 缓解：本次 MVP 不做 runtime schema validation。Builtin body 有 Rust 类型保障。Custom body 只在 SDK 扩展时出现，届时再加验证。

**[Risk] SessionState 新增字段对旧会话 replay 的影响**
→ 缓解：session_mode 默认 Execute，replay 旧会话时 `SessionState::new()` 使用默认值，不影响已有事件流。ModeChanged 事件只在模式切换时产生，旧会话不存在此事件。

**[Trade-off] 双层 Artifact 增加了序列化/反序列化复杂度**
→ 接受：Ref 和 Body 的使用场景确实不同（事件流 vs LLM 消费），合并成一个结构会导致"为了事件轻量而限制 Body 内容"或"为了 Body 丰富而让事件流膨胀"。拆开后各司其职。

## Open Questions

1. **switchMode tool 的返回格式**：当 LLM 在 plan mode 调用 `switchMode("execute")` 时，tool result 应该返回什么？纯文本提示？还是结构化的"等待用户确认"状态？如果 execute 的 entry_policy 是 UserOnly，LLM 调用被拒绝时怎么反馈？

2. **PlanArtifact 的 PlanContent schema**：steps 的结构化程度——是自由文本步骤列表，还是强类型 `{ description, files, risk }` 数组？强类型方便 UI 渲染，但 LLM 生成的准确性存疑。

3. **ModeMap prompt block 的缓存策略**：ModeMap（列出所有可用 mode）属于 SemiStable 层还是 Dynamic 层？如果自定义 mode 动态注册/卸载，ModeMap 需要每次 turn 重新生成。

4. **子 agent 是否继承父 agent 的 mode**：当父 agent 在 plan mode 下 spawn 子 agent，子 agent 应该是 execute mode（默认）还是继承 plan mode？建议默认 execute（子 agent 有自己的 scoped router），但需确认。
