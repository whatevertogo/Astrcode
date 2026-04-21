# Astrcode 声明式 DSL 与编译器目标架构说明书

## 文档定位

本文档定义 Astrcode 在声明式 DSL、编译 IR 与运行时绑定方面的目标架构，用于统一后续 mode、workflow、prompt、policy 相关演进的术语、模块边界与重构顺序。

本文档是 `PROJECT_ARCHITECTURE.md` 在“声明式治理与编排”方向上的专项展开。若两者冲突，以 `PROJECT_ARCHITECTURE.md` 的仓库级分层边界为准；本文档负责把这些边界落实到 DSL、编译器与 IR 设计。

## 背景

Astrcode 当前已经具备较强的声明式架构基础，但“DSL”和“编译器”两个词在实现中承载了多种不同含义：

- `CapabilitySpec` 是运行时能力语义真相，不是普通配置项。
- `GovernanceModeSpec` 是治理 DSL，描述能力表面、策略、child 继承与 prompt program。
- `WorkflowDef` 是正式工作流 DSL，描述跨 turn 的 phase、signal、transition 与 bridge。
- `PromptDeclaration` 是稳定的 prompt 注入 DTO，而 `adapter-prompt` 又有 contributor/composer 这套编程式 prompt 管线。
- `compile_mode_envelope()` 已经在做 mode 编译，但 `GovernanceSurfaceAssembler` 还在继续补齐 prompt、policy、approval、busy policy、runtime 限制，导致“编译完成”与“运行时绑定完成”的边界不够清晰。

结果是：系统实际已经是“多 DSL + 多阶段编译”，但在命名、模块边界和 IR 层次上还没有形成统一语言。

同时，当前最紧迫的问题并不只是术语混乱，而是 `GovernanceModeSpec` 的表达能力还不足以支撑真正插件化的 mode 定义。尤其是 `plan` mode 仍然依赖硬编码工具、硬编码 artifact 语义和硬编码退出门，这使“目标架构”必须同时回答两件事：

- 长期上，如何统一声明式编译骨架；
- 短期上，如何先补齐 mode spec 的表达能力，让插件能够定义完整 mode。

## 设计目标

### 目标

1. 统一 Astrcode 内所有“声明式模型 -> 编译 -> 绑定 -> 执行”的术语和分层。
2. 明确 capability、mode、workflow、prompt 各自的职责，不再把它们混称为同一个 DSL。
3. 建立显式的 IR 分层，避免纯编译逻辑与 turn/session 绑定逻辑继续交织。
4. 让后续扩展可以沿着固定骨架演进：
   - 定义声明模型
   - 校验与归一化
   - 编译为纯 IR
   - 绑定成可执行快照
   - 交给 runtime 执行
5. 为后续可能的外部声明文件化保留空间，但不把“外置格式”当作当前阶段的首要目标。

### 非目标

- 不在本阶段把所有 DSL 外置成 YAML/JSON/TOML 文件。
- 不把 mode 与 workflow 强行合并为单一 DSL。
- 不把 `adapter-prompt` contributor 体系改造成完全数据驱动。
- 不改变 `PROJECT_ARCHITECTURE.md` 已经确定的仓库级分层方向。

## 当前系统定位

### 一、语义基座

`CapabilitySpec` 是运行时内部唯一的 capability semantic truth，定义于 `core`，服务于 router、policy、prompt、plugin、governance 的统一判断。

当前价值：

- 为 `CapabilitySelector` 提供统一选择语义。
- 避免 runtime 内出现并行 capability registry。
- 使工具、副作用、标签、权限、稳定性等判断都能围绕同一模型展开。

结论：

- `CapabilitySpec` 应被视为“语义模型层”，而不是“声明 DSL 的一个普通分支”。

### 二、治理声明层

`GovernanceModeSpec` 是治理 DSL，负责回答“这一轮允许做什么、如何做、对子代理如何收缩”。

它当前包含：

- capability selector
- action policies
- child policy
- execution policy
- prompt program
- transition policy

结论：

- mode 是治理约束 DSL，不是 workflow DSL。
- mode 编译的结果应是“纯治理 IR”，而不是最终 turn 可执行快照。

### 三、工作流声明层

`WorkflowDef` 是 workflow DSL，负责回答“当前处于正式流程的哪一段、如何迁移、迁移时桥接什么上下文”。

它当前包含：

- phase
- transition
- signal
- bridge state envelope

结论：

- workflow 是正式编排 DSL，独立于 mode。
- workflow 复用 mode，但不重建 mode catalog，也不篡改 capability 语义层。

### 四、prompt 声明与编程式 prompt 管线

当前 prompt 相关内容存在两条并行路径：

- 声明式路径：`PromptDeclaration`
- 编程式路径：contributor/composer

结论：

- `PromptDeclaration` 应被定义为“稳定 prompt 注入协议”。
- contributor/composer 不应被误称为 DSL 本体，更适合定义为“prompt 标准库与组装器”。

### 五、编译与绑定的现状问题

当前主要边界如下：

- mode 编译：`GovernanceModeSpec -> 编译期治理产物（当前命名仍为 ResolvedTurnEnvelope）`
- governance 绑定：`编译期治理产物 + runtime/session/control -> ResolvedGovernanceSurface`
- workflow 编排：`WorkflowDef + persisted state + signal -> next workflow state`

问题不在于实现方向错误，而在于这几个阶段没有被统一成同一套编译语言：

- `ResolvedTurnEnvelope` 当前命名容易让人误解为最终执行快照，但它的语义更接近“治理编译产物”。
- `ResolvedGovernanceSurface` 才是 bind 完成后供 runtime 一次性消费的治理快照。
- workflow 现在更像“声明 + orchestrator”，缺少一个显式 compile/normalize 层。
- prompt program 有一部分在 mode spec 里，一部分在 assembler helper 里，语义上不够收敛。

### 六、插件声明与消费路径

当前插件 DSL 的注册入口已经存在，但文档化不足：

- 插件通过 `InitializeResultData` 声明 `capabilities`、`skills`、`modes`
- server bootstrap / reload 路径把这些声明分别接入 capability surface、skill catalog、mode catalog
- 后续 turn 才会在 governance 编译阶段消费 plugin mode

这意味着 Astrcode 的“声明式 DSL”并不只是 core 里的 struct 定义，还包括一条完整的 host 消费链：

```text
plugin InitializeResultData
    -> bootstrap / reload
    -> CapabilitySurface / SkillCatalog / ModeCatalog
    -> governance compile / bind
    -> runtime execution
```

结论：

- 任何 mode DSL 演进都必须同时考虑 host 注册路径与 reload 语义。
- 只改 `GovernanceModeSpec` 而不分析 plugin 消费路径，会低估变更影响面。

### 七、选择器求值的核心地位

`CapabilitySelector` 的递归求值是当前 mode compiler 最核心的逻辑之一。

它不仅决定 mode 的 allowed tools，还直接参与：

- child capability 收缩
- grant 进一步裁剪
- subset router 构造

结论：

- selector evaluation 不是“编译中的一个小步骤”，而是 mode compiler 的核心算法面。
- 后续如果引入更强的 mode spec 表达力，应优先保证 selector 语义保持稳定、可测、可复用。

### 八、当前最紧迫的扩展性瓶颈

在当前代码状态下，最紧迫的问题不是 workflow 索引化或 prompt IR 命名，而是 `GovernanceModeSpec` 仍不足以表达完整 mode 生命周期。

主要缺口包括：

- 缺少 mode 级 artifact 定义，导致 `plan` 依赖 `upsertSessionPlan`
- 缺少 mode 级退出门定义，导致 `exitPlanMode` 逻辑硬编码
- 缺少 mode 级动态 prompt hook，导致 mode 行为依赖 builtin helper 和固定 prompt 文案
- 工具侧还拿不到稳定的 mode contract snapshot，导致 artifact / exit / prompt 合同只能散落在 builtin plan 逻辑里

结论：

- “统一编译骨架”仍然重要，但短期优先级应让位于“补齐 `GovernanceModeSpec` 的表达能力”。
- 目标架构必须把这条主线纳入第一优先级，而不是作为后续扩展再讨论。

## 目标架构总览

目标架构统一采用四层模型：

1. 语义模型层
2. 声明层
3. 编译 IR 层
4. 绑定执行层

对应关系如下：

```text
CapabilitySpec / Policy Types / PromptDeclaration DTO / Workflow DTO
    -> GovernanceModeSpec / WorkflowDef
    -> Compiled Governance IR / Compiled Workflow IR
    -> ResolvedGovernanceSurface / ResolvedWorkflowState
    -> session-runtime execution
```

更具体地说：

```text
CapabilitySpec
    -> GovernanceModeSpec
        -> CompiledModeSurface
            -> ResolvedGovernanceSurface

WorkflowDef
    -> CompiledWorkflowPlan
        -> BoundWorkflowState
            -> application orchestration

PromptDeclaration + Prompt contributors
    -> bound prompt inputs
        -> PromptPlan
            -> prompt composer / model submission
```

## 模块边界

### `core`

`core` 继续作为语义契约层，负责：

- `CapabilitySpec`
- `GovernanceModeSpec`
- `WorkflowDef`
- `PromptDeclaration`
- policy / approval / prompt / workflow 的稳定 DTO

`core` 只定义声明协议与稳定数据模型，不承担 application 层的装配、绑定与运行时上下文解析。

补充约束：

- workflow artifact 持有 `phase.mode_id`，继续作为 phase -> mode 绑定的唯一 owner。
- `core` 可以定义 mode contract 的纯 DTO，但不得因此把 workflow owner 反向塞回 mode spec。

### `application::governance`

建议把当前 mode compiler + governance surface assembler 逐步收敛为一个更清晰的治理子域：

- `spec`：治理声明入口与 catalog
- `compiler`：纯编译
- `binder`：turn/session/runtime 绑定
- `surface`：可执行治理快照

职责边界：

- 编译器只处理 `spec -> IR`
- binder 只处理 `IR + runtime inputs -> executable surface`
- surface 是 runtime 与 prompt submission 的唯一消费入口

### `application::workflow`

建议把 workflow 子域明确拆为：

- `definition`：builtin workflow 声明
- `compiler`：workflow 归一化与编译
- `orchestrator`：基于 compiled workflow 做 signal / transition / persistence
- `state`：持久化状态与 bridge state 服务

职责边界：

- workflow compiler 不解释 session-runtime 事实
- orchestrator 不承担 mode 编译职责
- workflow 只决定业务 phase，不直接决定 capability surface

### `adapter-prompt`

建议明确其角色为：

- prompt rendering / composition 基础设施
- prompt contributor 标准库
- prompt declaration 的渲染与排序执行器

不再把它描述成“另一个 DSL 编译器”；它消费上游已经绑定好的 prompt 输入。

## 统一命名方案

### 一、术语规范

- `semantic model`
  指运行时稳定语义真相，例如 `CapabilitySpec`
- `spec`
  指声明模型，例如 `GovernanceModeSpec`、`WorkflowDef`
- `compile`
  指纯函数、无 session/runtime 实例状态参与的声明到 IR 转换
- `normalize`
  指在 compile 前做的结构校验、默认值填充、去重、显式化步骤
- `bind`
  指把 IR 与 turn/session/runtime/profile/control 组合成可执行快照
- `surface`
  指绑定完成、可直接被 runtime 或 prompt submission 消费的对象
- `orchestrate`
  指根据 workflow state、signal、bridge 做业务迁移

补充：

- 当前代码中的 `ResolvedTurnEnvelope` 仍保留旧名字，但本文统一把它视为 compile 层产物。
- 当前代码中的 `ResolvedGovernanceSurface` 是 bind 层结果，两者不得再混称为同一层 envelope。

### 二、建议重命名

| 当前名称 | 建议名称 | 原因 |
|---|---|---|
| `ResolvedTurnEnvelope` | `CompiledGovernanceEnvelope` 或 `CompiledModeSurface` | 它更像编译后的治理 IR，而不是最终 resolved surface |
| `compile_mode_envelope()` | `compile_mode_surface()` | 与目标概念一致 |
| `CompiledModeEnvelope` | `CompiledGovernanceSurface` 或 `CompiledModeArtifact` | 避免 envelope / surface 双重混用 |
| `GovernanceSurfaceAssembler` | `GovernanceSurfaceBinder` | 更准确表达它的工作是运行时绑定 |
| `build_surface()` | `bind_surface()` | 与 compile/bind 两阶段配套 |
| `WorkflowOrchestrator` | 保持不变 | 它确实承担编排职责，不应误称 compiler |

说明：

- 若短期内不希望大规模重命名，可以先通过注释与模块文档显式定义语义，再逐步重命名。
- 最需要先统一的是“compiled IR”和“bound surface”这两个层次。

## IR 设计

### 一、治理 IR

建议引入明确的治理编译 IR，目标形状如下：

```text
GovernanceModeSpec
    -> CompiledModeSurface
    -> BoundGovernanceSurface
```

说明：

- 当前不强制新增公开的 `NormalizedModeSpec` 类型。
- `GovernanceModeSpec::validate()` 已经覆盖基础校验，短期可以继续沿用。
- 若后续确实出现默认值展开、plugin merge、来源标记补全等需求，可在 compiler 内部引入 normalize 阶段，但不应把“新增 normalize 层”作为当前重构前提。

#### `CompiledModeSurface`

职责：

- 表达纯治理语义，不绑定 turn/session/runtime
- 保存 capability surface 与 policy surface 的编译结果
- 成为 binder 的稳定输入

建议字段：

- `mode_id`
- `allowed_tools`
- `capability_router_delta` 或 subset 描述
- `compiled_action_policies`
- `compiled_child_policy`
- `compiled_prompt_program`
- `compiled_execution_policy`
- `diagnostics`

说明：

- 若 `CapabilityRouter` 需要依赖 runtime registry，IR 里可以先保存“subset description”而非最终 router 实例。
- `PromptDeclaration` 仍可作为 prompt program 的目标 DTO，但“这是 mode 直接声明的 prompt”应被保留为显式来源信息。
- 更重要的是，后续 mode spec 扩展应优先把 artifact、exit gate、prompt hooks 这些能力收进 spec，再由 compiler 产出对应 IR。
- phase -> mode 绑定继续由 workflow artifact 持有；治理 compiler 只消费 mode id，不反向声明 workflow 所有权。

#### `BoundGovernanceSurface`

这就是当前 `ResolvedGovernanceSurface` 的目标定位，也是 governance snapshot 的唯一 bind owner。

职责：

- 合并 runtime config、execution control、turn/session/profile
- 构造最终 `PolicyContext`
- 注入协作 prompt、child 合同 prompt、submission skill prompt
- 生成 approval pipeline
- 形成 runtime 一次性消费的治理快照

建议保留：

- `runtime`
- `capability_router`
- `prompt_declarations`
- `resolved_limits`
- `policy_context`
- `approval`
- `busy_policy`
- `diagnostics`

### 二、workflow IR

建议引入 workflow 编译 IR，目标形状如下：

```text
WorkflowDef
    -> CompiledWorkflowPlan
    -> BoundWorkflowState
```

#### `CompiledWorkflowPlan`

职责：

- 为 orchestrator 提供无歧义、可校验的运行结构
- 显式承载 workflow 校验和 phase/transition 查询语义

建议字段：

- `workflow_id`
- `initial_phase_id`
- `phases`
- `transitions`
- `bridge_contracts`
- `diagnostics`

说明：

- 当前阶段不要求为了 compile artifact 专门引入索引化 `HashMap`。
- 在现有 workflow 规模下，保留 `Vec` 结构完全可以接受。
- “显式 compiled workflow artifact”与“索引化优化”不是同一件事，前者优先，后者按规模决定。

#### `BoundWorkflowState`

职责：

- 把 persisted workflow state 与 compiled workflow plan 对齐
- 形成当前 active phase 的绑定结果
- 供 application 用例编排消费

建议字段：

- `workflow_id`
- `current_phase`
- `bound_mode_id`
- `artifact_refs`
- `bridge_state`
- `allowed_signals`
- `diagnostics`

### 三、prompt 结果模型

prompt 不建议再凭空新增一套与 `PromptPlan` 重叠的公开 IR。

当前更合理的边界是：

```text
Prompt declarations + contributor outputs
    -> bound prompt inputs
    -> PromptPlan
```

说明：

- `adapter-prompt` 里的 `PromptPlan`、`PromptBlock`、`BlockMetadata` 已经承担了排序、来源、渲染目标、层级这些职责。
- 这里真正需要补齐的不是“再造一个 prompt IR 名字”，而是上游治理侧要把 prompt 的来源和绑定责任讲清楚。
- 因此本文后续统一使用“bound prompt inputs -> PromptPlan”这一表述。

## 目标编译链路

### 一、治理链路

```text
ModeCatalog
    -> load GovernanceModeSpec
    -> normalize
    -> compile to CompiledModeSurface
    -> bind with runtime/session/control/profile
    -> BoundGovernanceSurface
    -> AppAgentPromptSubmission / PolicyEngine / runtime
```

约束：

- normalize/compile 不读取 session state
- binder 不重新解释 selector 语义
- runtime 不再二次推导治理策略

### 二、workflow 链路

```text
builtin/plugin workflow defs
    -> normalize
    -> compile to CompiledWorkflowPlan
    -> load persisted workflow instance
    -> bind current phase state
    -> orchestrate signal/transition
    -> persist next workflow instance
```

约束：

- workflow 只负责编排和 phase 语义
- workflow 不直接生成 capability surface
- mode 仍通过 governance compiler/binder 独立生成

### 三、prompt 链路

```text
mode prompt program + governance prompt helpers + prompt facts + contributor outputs
    -> bind prompt inputs
    -> PromptPlan
    -> adapter-prompt compose/render
    -> model request
```

约束：

- governance 负责决定“应该注入什么”
- adapter-prompt 负责决定“如何组装与渲染”
- 工具执行只消费从 bound governance surface 投影出来的纯数据 mode contract snapshot，而不是直接依赖 application 内部类型。

## 并行推进方案

本文档不建议采用严格线性的“五阶段串行推进”。更合理的做法是围绕两条主线并行推进，再穿插两个支撑项。

### 主线 A：补齐 `GovernanceModeSpec` 的表达能力

目标：

- 先解决 mode 无法被插件完整定义的问题

动作：

- 为 `GovernanceModeSpec` 增加 mode 级 artifact 描述能力
- 为 `GovernanceModeSpec` 增加 exit gate 描述能力
- 为 `GovernanceModeSpec` 增加动态 prompt hooks 或等价扩展点
- 为工具链路补充 pure-data 的 bound mode contract snapshot
- 识别并收敛 `plan` mode 当前依赖的硬编码语义

预期收益：

- `plan` mode 的内建专有逻辑可以开始向通用 mode 机制迁移
- plugin mode 不再只能声明“工具白名单 + 提示词”，而能声明完整 mode 合同

### 主线 B：显式化 compile / bind 边界

目标：

- 让治理编译器和运行时绑定器的边界在代码与术语上都变清楚

动作：

- 把 `compile_mode_envelope()` 的产物显式定位为治理编译结果
- 把 `GovernanceSurfaceAssembler` 改名或语义收束为 binder
- 补齐模块注释，固定 compile / bind / orchestrate 术语
- 保证 binder 不再解释 selector，不再回流承担声明语义校验

预期收益：

- 后续新增 artifact / exit gate / prompt hook 时，不会继续把语义解释塞进 binder
- 相关类型与测试更稳定

### 支撑项 C：workflow 编译轻量化显式化

目标：

- 给 workflow 一条与治理链路一致的“声明 -> 校验/编译 -> 编排”骨架

动作：

- 为 `WorkflowDef` 增加显式 validate/compile 边界
- 保持当前 `Vec` 结构，不为索引化而索引化
- 让 `WorkflowOrchestrator` 只消费已校验的 workflow artifact

说明：

- 当前不把索引化视为必要前提。
- 这里的重点是边界清晰，而不是数据结构优化。

### 支撑项 D：prompt 来源与 metadata 收束

目标：

- 解决 prompt 来源模糊与 metadata 弱类型扩散问题

动作：

- 统一 mode prompt、协作 prompt、child 合同 prompt、skill 选择 prompt 的来源标记
- 明确 governance 负责决定“注入什么”，`adapter-prompt` 负责决定“如何渲染”
- 优先收紧高频 metadata 字段，把关键治理信息从匿名 JSON blob 中拿出来

## 目录与模块演进建议

本文档不要求立刻把现有目录拆成更多文件。当前更重要的是语义收束，而不是文件数量增长。

建议原则如下：

- 优先通过类型命名、模块注释和函数职责收束 compile / bind / orchestrate
- 只有在单文件同时承担多类职责时，才拆分物理文件
- `workflow` 子域优先补齐 validate/compile 语义，不强制提前重排目录
- `governance_surface` 现有文件数并不是问题，真正的问题是 binder/compile 语义混名

## 设计约束

后续实现必须满足以下约束：

1. mode 与 workflow 继续保持分离职责。
2. `CapabilitySpec` 继续是唯一 capability semantic truth。
3. `application` 负责 compile/bind/orchestrate，`session-runtime` 负责执行与事实。
4. prompt renderer 不承载治理语义真相。
5. binder 可以依赖 runtime/session/profile/control，compiler 不可以。
6. 所有 compiled artifact 都必须可单测、可序列化或至少可稳定断言其结构。
7. plugin 声明的 modes / capabilities / skills 在 reload 时必须满足一致性要求：要么原子切换，要么失败时完整回滚。
8. `CapabilitySelector` 的语义必须保持稳定，任何 mode spec 扩展都不能破坏其现有递归组合行为。
9. reload 继续遵守 idle-only 合同；不为 mixed-snapshot 引入额外执行模型。

## 验收标准

当以下条件同时满足时，可认为目标架构基本落地：

- 新代码中 compile/bind/orchestrate 三类职责不再混用。
- `GovernanceModeSpec` 已能表达 mode 级 artifact、exit gate、prompt hook 或等价扩展点。
- 治理链路存在显式 compiled artifact 与 bound surface。
- workflow 链路存在显式 compiled artifact，而不只是 `WorkflowDef + Orchestrator`。
- prompt block 来源可追踪，并明确沉淀到现有 `PromptPlan` 组装结果里。
- 关键治理路径中匿名 `metadata: Value` 的使用明显收敛。
- plugin reload 对 mode catalog、capability surface、skill catalog 的切换具备一致性保障。
- 新增内建或插件 mode / workflow 时，开发者可以按照统一骨架完成：
  - 定义 spec
  - compile
  - bind
  - verify

## 风险与注意事项

### 一、不要把“统一架构”误解成“统一 DSL”

mode、workflow、prompt、capability 不是同一种语义对象。统一的是编译骨架和术语，不是把它们压扁成一个超级 schema。

### 二、不要过早引入外部配置格式

在 IR 和 binder 边界尚未稳定前，把 spec 外置成文件只会把不清晰的内部结构序列化出去，反而固化问题。

### 三、不要让 mode 表达力问题被纯命名重构掩盖

如果 `GovernanceModeSpec` 仍不能表达 artifact、exit gate、动态 prompt hook，那么仅仅重命名 assembler/compiler 不会改善插件扩展能力。

### 四、不要让 binder 回流承担语义解释

一旦 binder 又开始解释 selector、补默认值、重写 workflow 规则，编译边界就会再次塌陷。

### 五、不要重复创造已经存在的 prompt 结果模型

`PromptPlan` 已经承担 prompt 组装结果的核心职责。后续需要做的是收束来源和绑定语义，而不是再造一个平行 prompt IR。

### 六、不要忽略 reload 一致性

如果 plugin mode 已更新、capability surface 未更新，或 skill catalog 已更新、mode catalog 回滚失败，就会产生事实漂移。重构必须把这一致性问题纳入第一批约束。

### 七、不要让 prompt 基础设施反向拥有治理真相

prompt renderer 只负责渲染与组合；“为何注入这些块”必须由 governance/application 决定。

## 推荐下一步

1. 先把本说明书对应到一个 OpenSpec change，正式管理重构范围。
2. 第一优先级推进 `GovernanceModeSpec` 扩展，把 artifact / exit gate / prompt hook 收进 spec，并为工具执行补上稳定的 mode contract snapshot。
3. 与此同时推进 compile / bind 术语显式化，避免新能力继续堆进 binder。
4. 再补 workflow validate/compile 边界与 reload 一致性约束。
5. 最后统一 prompt 来源标记与 metadata 类型化。

## 参考实现入口

- `PROJECT_ARCHITECTURE.md`
- `crates/core/src/capability.rs`
- `crates/core/src/mode/mod.rs`
- `crates/core/src/workflow.rs`
- `crates/core/src/ports.rs`
- `crates/application/src/mode/compiler.rs`
- `crates/application/src/mode/catalog.rs`
- `crates/application/src/governance_surface/mod.rs`
- `crates/application/src/governance_surface/assembler.rs`
- `crates/application/src/governance_surface/prompt.rs`
- `crates/application/src/workflow/orchestrator.rs`
- `crates/adapter-prompt/src/plan.rs`
- `crates/adapter-prompt/src/block.rs`
- `crates/protocol/src/plugin/handshake.rs`
- `crates/server/src/bootstrap/governance.rs`
- `crates/server/src/bootstrap/capabilities.rs`
- `openspec/specs/capability-semantic-model/spec.md`
- `openspec/specs/governance-mode-system/spec.md`
- `openspec/specs/mode-capability-compilation/spec.md`
- `openspec/specs/mode-policy-engine/spec.md`
- `openspec/specs/mode-prompt-program/spec.md`
- `openspec/specs/governance-surface-assembly/spec.md`
- `openspec/specs/workflow-phase-orchestration/spec.md`
