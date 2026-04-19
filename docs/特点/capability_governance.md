# Capability 抽象与治理面

Astrcode 将所有可被 Agent 调用的能力统一建模为 **CapabilitySpec**，并通过声明式 DSL（`CapabilitySelector`）在治理模式间灵活分配。这不是简单的"工具白名单"，而是一套从元数据声明到运行时执行的完整能力治理体系。

## 核心模型

### CapabilitySpec — 运行时能力语义定义

`CapabilitySpec`（`crates/core/src/capability.rs`）是运行时唯一的能力模型，每个工具、Agent、ContextProvider 都以 CapabilitySpec 描述自身：

```rust
pub struct CapabilitySpec {
    pub name: CapabilityName,         // 唯一标识，如 "readFile"
    pub kind: CapabilityKind,         // 类型：Tool / Agent / ContextProvider / ...
    pub description: String,          // 人类可读描述
    pub input_schema: Value,          // JSON Schema
    pub output_schema: Value,
    pub invocation_mode: InvocationMode,   // Unary / Streaming
    pub concurrency_safe: bool,            // 是否允许并发执行
    pub compact_clearable: bool,           // compaction 时是否可清除结果
    pub profiles: Vec<String>,             // 能力画像，如 "coding"
    pub tags: Vec<String>,                 // 自由标签，如 "filesystem", "read"
    pub permissions: Vec<PermissionSpec>,  // 声明的权限
    pub side_effect: SideEffect,           // 副作用级别
    pub stability: Stability,              // 稳定性级别
    pub metadata: Value,                   // 扩展元数据
    pub max_result_inline_size: Option<usize>,  // 结果内联阈值
}
```

构造通过类型安全的 Builder 完成（`CapabilitySpecBuilder`），`build()` 时校验所有不变量（名称非空、schema 必须是 JSON Object、无重复标签/权限等）。

### 关键枚举

| 类型 | 值 | 作用 |
|------|-----|------|
| `CapabilityKind` | Tool, Agent, ContextProvider, MemoryProvider, PolicyHook, Renderer, Resource, Prompt, Custom | 能力类型判别 |
| `SideEffect` | None, Local, Workspace, External | 副作用分级，治理面的核心轴 |
| `Stability` | Experimental, Stable, Deprecated | API 成熟度 |
| `InvocationMode` | Unary, Streaming | 调用模式 |

其中 `SideEffect` 是治理模式收缩工具面的主要维度：

- **None** — 纯读操作，无副作用（如 readFile、grep）
- **Local** — 写入本地文件系统（如 writeFile、editFile）
- **Workspace** — 修改工作区状态（大部分内置工具的默认值）
- **External** — 影响外部系统（如 shell）

### 协议层映射

`CapabilityWireDescriptor`（`crates/protocol/src/capability/descriptors.rs`）是协议层对 `CapabilitySpec` 的传输别名，直接 re-export：

```rust
pub use astrcode_core::CapabilitySpec as CapabilityWireDescriptor;
```

不再维护第二个语义冗余的类型。插件 SDK 的 `capability_mapping.rs` 提供恒等转换（`wire_descriptor_to_spec` / `spec_to_wire_descriptor`），只做 validate + clone。

## 工具如何声明能力

### 三层声明链

```
Tool trait（core）
  ├─ fn definition() -> ToolDefinition          // 名称、描述、参数 schema
  ├─ fn capability_metadata() -> ToolCapabilityMetadata  // 能力元数据
  └─ fn capability_spec() -> CapabilitySpec     // 合并产物（默认实现）
```

`ToolCapabilityMetadata`（`crates/core/src/tool.rs`）携带能力语义：

```rust
pub struct ToolCapabilityMetadata {
    pub profiles: Vec<String>,
    pub tags: Vec<String>,
    pub permissions: Vec<PermissionSpec>,
    pub invocation_mode: InvocationMode,
    pub side_effect: SideEffect,
    pub concurrency_safe: bool,
    pub compact_clearable: bool,
    pub stability: Stability,
    pub prompt: Option<ToolPromptMetadata>,
    pub max_result_inline_size: Option<usize>,
}
```

默认值通过 `builtin()` 提供：profile `"coding"`、tag `"builtin"`、`SideEffect::Workspace`、`Stability::Stable`。

### 内置工具的能力声明示例

| 工具 | tags | side_effect | concurrency_safe | 权限 |
|------|------|-------------|-----------------|------|
| `readFile` | filesystem, read | None | true | filesystem.read |
| `writeFile` | filesystem, write | Local | false | filesystem.write |
| `editFile` | filesystem, write, edit | Local | false | filesystem.write |
| `grep` | filesystem, read, search | None | true | filesystem.read |
| `shell` | process, shell | External | false | shell.exec |

### 注册到内核

`ToolCapabilityInvoker`（`crates/kernel/src/registry/tool.rs`）将 `dyn Tool` 包装为 `dyn CapabilityInvoker`，在构造时调用 `tool.capability_spec()` 获取并缓存 spec。`CapabilityRouter`（`crates/kernel/src/registry/router.rs`）维护 `HashMap<String, Arc<dyn CapabilityInvoker>>` 注册表。

## CapabilitySelector — 声明式能力选择 DSL

`CapabilitySelector`（`crates/core/src/mode/mod.rs`）是一个递归代数 DSL：

```rust
pub enum CapabilitySelector {
    AllTools,                                    // 全部 Tool 类型能力
    Name(String),                                // 按名称精确匹配
    Kind(CapabilityKind),                        // 按 CapabilityKind 过滤
    SideEffect(SideEffect),                      // 按副作用级别过滤
    Tag(String),                                 // 按标签过滤
    Union(Vec<CapabilitySelector>),              // 集合并
    Intersection(Vec<CapabilitySelector>),       // 集合交
    Difference { base: Box, subtract: Box },     // 集合差
}
```

求值逻辑在 `evaluate_selector()`（`crates/application/src/mode/compiler.rs`）：从全量 `CapabilitySpec` 列表开始，递归匹配，最终产出 `BTreeSet<String>`（工具名集合）。

这个 DSL 让治理模式可以用声明式表达式定义自己的能力面，而不需要硬编码工具列表。

### 内置模式的能力面定义

**Code 模式** — 完全访问：

```rust
capability_selector: CapabilitySelector::AllTools
```

**Plan 模式** — 只读 + 两个计划工具：

```rust
capability_selector: CapabilitySelector::Union(vec![
    // 全部工具 - 副作用工具 - agent 标签
    Difference {
        base: AllTools,
        subtract: Union([SideEffect(Local), SideEffect(Workspace),
                         SideEffect(External), Tag("agent")])
    },
    // 显式加入计划专属工具
    Name("exitPlanMode"),
    Name("upsertSessionPlan"),
])
```

**Review 模式** — 严格只读：

```rust
capability_selector: CapabilitySelector::Intersection(vec![
    AllTools,
    SideEffect(None),          // 只允许 SideEffect::None 的工具
    Difference { base: AllTools, subtract: Tag("agent") },
])
```

## 编译与装配：从声明到运行时

### 完整生命周期

```
工具注册
  Tool::capability_metadata() + Tool::definition()
    → ToolCapabilityMetadata::build_spec()
      → CapabilitySpec (validated)
        → ToolCapabilityInvoker 包装为 CapabilityInvoker
          → CapabilityRouter::register_invoker()

                ↓

Turn 提交时编译治理面
  GovernanceSurfaceAssembler.session_surface()
    → compile_mode_surface()
      → compile_mode_envelope(base_router, mode_spec, extra_prompts)
        → compile_capability_selector()  // 递归求值 CapabilitySelector
          → evaluate_selector()  → BTreeSet<String>  allowed_tools
        → child_allowed_tools()  // 计算子代理继承的工具白名单
        → subset_router()  // 创建过滤后的 CapabilityRouter
        → ResolvedTurnEnvelope  // 编译产物
    → build_surface()
      → ResolvedGovernanceSurface  // 最终运行时治理面

                ↓

运行时执行
  execute_tool_calls()
    → 查询 capability_spec 判断 concurrency_safe
    → safe 调用并发执行（buffer_unordered）
    → unsafe 调用串行执行
    → CapabilityRouter::execute_tool()  // 只有白名单内的工具可被调用
```

### ResolvedTurnEnvelope — 编译产物

`ResolvedTurnEnvelope`（`crates/core/src/mode/mod.rs`）是模式编译的输出：

```rust
pub struct ResolvedTurnEnvelope {
    pub mode_id: ModeId,
    pub allowed_tools: Vec<String>,               // 编译后的工具白名单
    pub prompt_declarations: Vec<PromptDeclaration>,
    pub action_policies: ActionPolicies,           // Allow / Deny / Ask 策略
    pub child_policy: ResolvedChildPolicy,         // 子代理策略
    pub submit_busy_policy: SubmitBusyPolicy,      // BranchOnBusy / RejectOnBusy
    pub fork_mode: Option<ForkMode>,
    pub diagnostics: Vec<String>,
}
```

### ResolvedGovernanceSurface — 运行时治理面

`ResolvedGovernanceSurface`（`crates/application/src/governance_surface/mod.rs`）是装配器输出的最终产物，携带：

- `capability_router` — 过滤后的路由器（只含 allowed_tools）
- `resolved_limits` — 工具白名单 + max_steps
- `policy_context` — 策略引擎上下文
- `approval` — 审批管线（Ask 模式时触发）
- `prompt_declarations` — 本 turn 的所有 prompt 块

它通过 `into_submission()` 转为 `AppAgentPromptSubmission`，传入 session runtime 的 turn 执行。

### 子代理的能力继承

`compile_mode_envelope_for_child()`（`crates/application/src/mode/compiler.rs`）计算子代理的可用工具：

```
子代理工具 = parent_allowed_tools ∩ mode_allowed_tools ∩ SpawnCapabilityGrant
```

三层取交集确保子代理的能力严格收缩，不会超出父代理和模式定义的范围。

## 运行时强制执行

### 并发安全调度

`execute_tool_calls()`（`crates/session-runtime/src/turn/tool_cycle.rs`）根据 `concurrency_safe` 字段分两路：

```
LLM 返回 tool_calls
    │
    ├─ concurrency_safe=true  →  buffer_unordered 并发执行
    │   （如 readFile、grep）
    │
    └─ concurrency_safe=false  →  串行执行
        （如 writeFile、shell）
```

### 工具路由的过滤生效

`CapabilityRouter::subset_for_tools_checked()` 创建一个只含白名单工具的新路由器。LLM 请求的 tool call 若不在白名单中，`execute_tool()` 查找失败返回 "unknown tool" 错误。这是**编译期过滤 + 运行时兜底**的双重保障。

### 策略引擎检查点

Policy Engine（`crates/core/src/policy/engine.rs`）提供两个检查点：

- `check_model_request()` — 可重写 LLM 请求（过滤工具列表、修改 system prompt）
- `check_capability_call()` — 对每次能力调用返回 Allow / Deny / Ask

## 设计优势

### 1. 声明式 vs 命令式：模式定义零代码

传统做法是在代码里写 `if mode == "plan" { disable_write_tools() }`。Astrcode 用 `CapabilitySelector` DSL 让模式定义完全声明化：

```
Plan 模式的工具面 =
    (AllTools - SideEffect(Local|Workspace|External) - Tag("agent"))
    ∪ Name("exitPlanMode") ∪ Name("upsertSessionPlan")
```

添加新模式只需写 spec，不需要改编译器或运行时代码。求值逻辑完全通用。

### 2. 元数据驱动：能力自描述

每个工具通过 `ToolCapabilityMetadata` 声明自己的语义属性（副作用级别、并发安全性、权限需求），而不是由治理层硬编码"哪些工具危险"。

这意味着：
- 新增工具时，只需声明自己的元数据，自动被所有模式的 `CapabilitySelector` 覆盖
- 修改工具的副作用级别会自动反映到所有引用该级别的模式
- 不需要维护"危险工具列表"这种容易过时的集中配置

### 3. 编译期求值：每次 turn 重新编译，动态生效

工具注册表可能因为插件加载/卸载而变化。`CapabilitySelector` 在每个 turn 提交时重新求值（`compile_mode_envelope`），而非在启动时静态绑定。

这确保了：
- 插件热加载后，新模式 spec 立刻生效
- 运行时修改 mode spec 不需要重启
- 编译产物（`ResolvedGovernanceSurface`）是一次性的，不会被旧状态污染

### 4. 子代理能力严格收缩

子代理的能力 = 父 ∩ 模式 ∩ 显式授权（`SpawnCapabilityGrant`），三层取交集。这确保了：

- 子代理永远不会获得超出父代理的能力
- 模式定义的全局约束不会被 spawn 请求绕过
- 显式授权进一步收缩，实现最小权限原则

### 5. CapabilitySpec 的多维元数据支撑多种运行时决策

同一份 `CapabilitySpec` 驱动多个独立的运行时决策：

| 元数据字段 | 消费者 | 决策 |
|-----------|--------|------|
| `side_effect` | CapabilitySelector | 模式的工具面收缩 |
| `concurrency_safe` | tool_cycle | 并行 vs 串行执行 |
| `compact_clearable` | compaction | compaction 时是否可清除结果 |
| `tags` | CapabilitySelector / prompt contributor | 按标签过滤、prompt 注入 |
| `permissions` | policy engine | 权限检查 |
| `max_result_inline_size` | tool execution | 结果持久化阈值 |
| `stability` | prompt contributor | 向用户展示 API 成熟度 |

不需要为每个决策维度维护独立的数据源。

### 6. 单一规范模型的边界清晰

`CapabilitySpec` 在 `core` 层定义，`CapabilityWireDescriptor` 在 `protocol` 层只是 re-export。不存在两套互相转换的模型。插件 SDK 的映射层是恒等转换。

这避免了"core 模型和 protocol 模型不一致"的经典问题，同时保持了 crate 依赖方向的正确性（protocol 依赖 core，不是反过来）。

## 与同类产品的对比

### Claude Code

Claude Code 没有独立的 capability 抽象。工具以 `Entry`（entry.ts）的形式存储在 JSONL 中，没有结构化的元数据（副作用级别、并发安全性、权限声明）。权限控制通过硬编码的 permission mode（`default`、`plan`、`auto`）在 `AgentLoop` 中判断。

工具过滤直接在 prompt 组装时修改 `tools` 数组，没有编译/求值阶段。

### Codex

Codex 的工具定义在 `AgentFunctionDefinition` 中（`agent-functions.ts`），包含 `name`、`description`、`parameters`，但没有副作用分级、并发安全性、权限声明等治理元数据。

工具白名单通过 `allowedTools` 数组在 `applyPolicy()` 中硬编码过滤，是命令式的列表匹配。

### 对比总结

| 维度 | Astrcode | Claude Code | Codex |
|------|----------|-------------|-------|
| 能力模型 | 结构化 `CapabilitySpec`（15+ 字段） | Entry 消息，无独立模型 | `AgentFunctionDefinition`，字段较少 |
| 元数据维度 | 副作用级别、并发安全、权限、稳定性、标签 | 无 | 无 |
| 模式定义 | 声明式 DSL（`CapabilitySelector`） | 硬编码 permission mode | 硬编码 `allowedTools` 列表 |
| 工具过滤 | 编译期求值 + 运行时路由 | prompt 组装时改 tools 数组 | `applyPolicy()` 过滤 |
| 子代理控制 | 三层交集收缩（父 ∩ 模式 ∩ 授权） | 无子代理概念 | 无子代理概念 |
| 新增工具的治理成本 | 声明元数据即可，自动被 DSL 覆盖 | 需要改 permission mode 逻辑 | 需要改 `allowedTools` 列表 |

本质区别：Astrcode 的 capability 是**自描述的能力单元**，治理层通过声明式规则组合它们；Claude Code 和 Codex 的工具是**扁平的函数定义**，治理层通过命令式代码管理它们。
