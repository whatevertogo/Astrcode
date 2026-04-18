---
name: collaboration-mode-system
created: "2026-04-18"
status: proposal
---

## Why

AstrCode 目前缺少显式的"协作阶段"建模。当用户说"先别动代码，只做方案"时，系统无法在结构层面响应——只能靠提示词软约束。与此同时，Claude Code 已将 Plan Mode 做成权限模式之一（只读分析 + 先给方案），Codex 也以 approval modes 分层控制执行摩擦。

当前代码里"模式"能力实际散落在多处：
- `AgentProfile.allowed_tools` — 工具授权（身份级）
- `CapabilityRouter.subset_for_tools()` — 工具过滤（子 agent 用）
- `PolicyEngine.check_capability_call()` — 运行时审批（Allow/Deny/Ask）
- `PromptDeclaration` — 提示词指导
- `AgentPromptSubmission` — 每 turn 的工具 + prompt + 执行限制（子 agent 包络雏形）

这些能力存在但缺少一个统一的概念：**协作模式（CollaborationMode）**。

核心问题：
1. **没有显式模式概念** — 无法区分"正在规划"和"正在执行"
2. **工具可见性和可执行性分裂** — LLM 能看到写工具，调用时才被 PolicyEngine 拒绝，浪费 token
3. **方案不是结构化对象** — Plan 只是 LLM 输出的一段文本，不可审批、不可版本化、不可失效
4. **模式不可扩展** — 无法让用户通过 SDK 自定义新模式

## What Changes

引入 **Mode System**，以 `ModeSpec` 为一等规格对象，统一控制工具授予、提示词注入、转换规则和产出协议。

### 核心概念

**五条正交轴（本次实现前三条）：**

| 轴 | 含义 | 真相归属 |
|---|---|---|
| Profile | 我是谁（稳定身份） | `AgentProfile` |
| **CollaborationMode** | **处于什么阶段** | **`SessionState.session_mode`** |
| ModeArtifact | 模式间的结构化交接 | `ModeArtifactRef` (durable) + `ModeArtifactBody` |
| ApprovalPolicy | 哪些动作要审批 | `PolicyEngine`（已有，渐进增强） |
| SandboxProfile | 进程/文件/网络边界 | 未来 |

### 三层架构

```
core          ModeSpec / CollaborationMode 枚举 / ModeArtifact / ToolGrantRule / ArtifactRenderer trait
              ← 只放稳定语义词汇，不放 runtime 编排细节

session-runtime
              SessionState.session_mode        ← durable 真相
              compile_mode_spec()              ← 编译 ModeSpec → visible_tools + prompt_directives
              apply_mode_transition()          ← 统一切换入口（tool/command/UI 快捷键汇聚）
              ModeArtifactStore                ← artifact 持久化与查询

application   BuiltinModeCatalog              ← plan/execute/review 的 ModeSpec 注册
              // TODO: PluginModeCatalog       ← 未来 SDK 自定义 mode
```

### Builtin Modes

**Plan 模式：**
- 工具授予：只读工具（readFile, grep, findFiles, listDir, toolSearch, webSearch）
- 提示词：强调只读分析、结构化方案、不修改代码
- 进入策略：`LlmCanEnter` — LLM 遇到复杂/不确定任务时可自行进入
- 产出：`ModeArtifact { kind: "plan" }` — 结构化方案对象

**Execute 模式（默认）：**
- 工具授予：全部
- 提示词：完整执行
- 进入策略：默认模式 / 用户确认后切换
- 产出：无

**Review 模式：**
- 工具授予：只读工具
- 提示词：代码审查
- 进入策略：`LlmCanEnter` — 用户要求 review 时自动进入
- 产出：`ModeArtifact { kind: "review" }`

### 模式切换机制

- **switchMode tool**：LLM 在 step 中调用，请求切换模式
- **/mode \<name\> command**：用户终端输入
- **Shift+Tab 快捷键**：UI 操作
- 全部汇聚到 `apply_mode_transition()`，验证转换合法性 + entry_policy

### 模式粒度：Turn 持久 / Step Runtime Override

- **Turn 级**：`SessionState.session_mode` 持久化，工具集在 turn 开始时编译，不可变
- **Step 级**：`TurnExecutionContext.step_mode_override` 仅影响 prompt directives，不改变工具集，不持久化
- 理由：`TurnExecutionResources.tools` 在 turn 开始时确定（`runner.rs:157`），改它代价大；step 级 prompt override 已经可行（每 step 都调 `assemble_prompt_request`）

### ModeArtifact 双层模型

```
ModeArtifactRef          ← 轻量引用，走事件流、UI、审批
  artifact_id, source_mode, kind, status, summary

ModeArtifactBody         ← 完整负载，走 render_to_prompt() 给 LLM
  Plan(PlanContent)      ← 强类型
  Review(ReviewContent)  ← 强类型
  Custom { schema_id, schema_version, data: Value }  ← SDK 扩展
```

- **Ref** 用于事件持久化、UI 展示、compact summary
- **Body** 通过 `ArtifactRenderer` trait 渲染成 `PromptDeclaration` 给 LLM 消费
- 渲染分级：Summary（UI/compact）→ Compact（context 紧张）→ Full（context 充裕）

### 工具授予策略

采用"只给"模式而非"过滤"模式——LLM 只看到当前 mode 授予的工具，天然不知道其他工具存在。

```rust
pub enum ToolGrantRule {
    Named(String),           // 按名称精确匹配
    SideEffect(SideEffect),  // 按 side_effect 类别授予
    All,                     // 授予全部
}
```

Plan 模式可声明 `SideEffect(None)` 只拿纯只读工具，不用逐个列名字。与 `CapabilitySpec.side_effect` 字段对齐。

## Non-goals

- **不做** step 级工具切换（只在 turn 边界切换工具集，step 级仅影响 prompt）
- **不做** Phase 状态机（Explore → DraftPlan → AwaitPlanApproval → Execute → Verify → Done），先只做 Plan / Execute / Review 三态
- **不做** SandboxProfile（进程/文件/网络沙箱边界），留 TODO
- **不做** ReasoningEffort（思考深度旋钮），留 TODO
- **不做** CapabilityBudget（文件数/命令数上限），留 TODO
- **不做** PluginModeCatalog（SDK 自定义 mode），但类型设计预留扩展点
- **不做** 隐式意图识别（"先别动代码"自动切 plan），先只做显式切换

## Capabilities

### P1 — 核心模式系统

- `mode-spec`：core 新增 CollaborationMode 枚举、ModeSpec 结构、ToolGrantRule、ModeEntryPolicy、ModeTransition
- `mode-truth`：session-runtime SessionState 新增 session_mode 字段，通过 StorageEvent 持久化切换历史
- `mode-compile`：session-runtime 新增 `compile_mode_spec()` 编译 ModeSpec → visible tools + prompt directives
- `mode-switch-tool`：新增 switchMode builtin tool，LLM 可调用请求切换
- `mode-switch-command`：新增 /mode command 入口
- `mode-prompt`：新增 ModeMap prompt block（告诉 LLM 有哪些 mode、何时使用）+ CurrentMode prompt block（当前约束）
- `builtin-modes`：application 注册 plan/execute/review 三个 builtin ModeSpec
- `mode-catalog`：core 新增 ModeCatalog trait + BuiltinModeCatalog 实现

### P2 — ModeArtifact

- `artifact-types`：core 新增 ModeArtifactRef、ModeArtifactBody（含 PlanContent/ReviewContent/Custom）、ArtifactStatus
- `artifact-renderer`：core 新增 ArtifactRenderer trait + builtin PlanArtifactRenderer 实现
- `artifact-store`：session-runtime 管理 active_artifacts，通过 StorageEvent 持久化
- `artifact-prompt-injection`：execute mode 从 active_artifacts 中查找 plan artifact，注入 prompt

### P3 — 统一切换入口 + 审批流

- `mode-transition`：session-runtime 新增 `apply_mode_transition()` 统一入口
- `mode-transition-validation`：验证转换合法性（ModeSpec.transitions）+ entry_policy 检查
- `mode-ui-integration`：前端 Shift+Tab 快捷键 → API → apply_mode_transition
- `artifact-accept-flow`：plan artifact 的 Accept/Reject 状态转换

## Impact

**用户可见影响：**
- 新增 `/mode plan`、`/mode execute`、`/mode review` 命令
- Plan 模式下 LLM 只做分析不做修改，体验更安全
- LLM 可以在复杂任务时自行进入 Plan 模式
- 方案产出后用户可在 UI 中审批，确认后切换 Execute 执行

**开发者可见影响：**
- core 新增 `mode` 模块（CollaborationMode, ModeSpec, ToolGrantRule 等）
- session-runtime 的 `SessionState` 新增 `session_mode` 字段
- session-runtime 新增 `mode_transition.rs` 模块
- `TurnExecutionResources` 的 tools 编译逻辑从直接读 gateway 改为经 mode compile
- `AssemblePromptRequest` 新增 mode 相关 prompt declarations
- `StorageEventPayload` 新增 `ModeChanged`、`ModeArtifactCreated` 变体
- 新增 `switchMode` builtin tool

**架构影响：**
- 模式真相落在 session-runtime（符合"会随对话推进变化、影响后续 turn 行为、需要恢复/重放/审计"的判定标准）
- Profile 保持稳定身份不变，不被临时阶段污染
- PolicyEngine 保持现有职责，未来可渐进增强为 per-mode 策略
- 类型设计预留 SDK 扩展点（Custom artifact body、ModeCatalog trait）

## Migration

无破坏性迁移：
- 默认 mode 为 Execute，现有行为完全不受影响
- session_mode 字段为新增，旧会话 replay 时默认 Execute
- switchMode tool 为新增 builtin tool，不影响现有工具注册
- ModeArtifact 为新增存储类型，不影响现有事件流
