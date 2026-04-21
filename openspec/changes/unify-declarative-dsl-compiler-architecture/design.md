## Context

Astrcode 已经具备声明式治理与正式 workflow 的核心骨架，但当前实现存在两类问题同时叠加：

1. 声明式编译边界不够清晰
   - `GovernanceModeSpec` 在 `core` 中定义为治理 DSL，但 `compile_mode_envelope()` 与 `GovernanceSurfaceAssembler` 之间的职责边界没有被统一命名。
   - workflow 目前主要体现为 `WorkflowDef + WorkflowOrchestrator`，缺少明确的“已校验 / 已编译 workflow artifact”概念。
   - prompt 侧同时存在 `PromptDeclaration` 与 contributor/composer 两套路径，但上游治理层没有把“为什么注入这些 prompt”完全讲清楚。

2. mode spec 的表达能力不足
   - builtin `plan` mode 依赖 `upsertSessionPlan`、`exitPlanMode` 与 canonical session plan artifact 的硬编码约定。
   - 插件虽然已经能通过 `InitializeResultData.modes` 注册 mode，但当前 mode spec 还不足以描述 artifact 合同、退出门、动态 prompt hook 与 phase 绑定。
   - reload 路径会分别替换 mode catalog、capability surface、skill catalog，但失败时没有统一的一致性回滚契约。

这次 change 的目标不是“发明一个统一超级 DSL”，而是建立统一的声明式编译骨架，同时先补齐 `GovernanceModeSpec` 的缺口，使 mode 真正具备插件化扩展基础。

受影响的主要模块：

- `crates/core/src/mode/mod.rs`
- `crates/application/src/mode/*`
- `crates/application/src/governance_surface/*`
- `crates/application/src/workflow/*`
- `crates/protocol/src/plugin/handshake.rs`
- `crates/server/src/bootstrap/governance.rs`
- `crates/server/src/bootstrap/capabilities.rs`

与 `PROJECT_ARCHITECTURE.md` 的关系：

- 本次方案不改变 `mode envelope / workflow phase / application orchestration / session-runtime truth` 四层划分。
- 需要补充的是：把 `compile`、`bind`、`orchestrate` 三类职责明确映射到这套分层中，并把 plugin mode 注册与 reload 一致性纳入治理组合根的正式约束。

## Goals / Non-Goals

**Goals:**

- 统一治理链路中的 `compile`、`bind`、`orchestrate` 术语与职责边界。
- 扩展 `GovernanceModeSpec`，让 mode 能声明 artifact 合同、exit gate、动态 prompt hook 与 workflow 绑定。
- 明确 plugin mode 的 host 消费链路和 reload 一致性要求。
- 让 prompt 结果继续沉淀到现有 `PromptPlan`，避免引入平行 prompt IR。
- 为 workflow 引入轻量的 validate/compile 语义，但保持当前规模下的实现克制。

**Non-Goals:**

- 不把 mode、workflow、prompt、capability 合并成单一 schema。
- 不在本次引入新的外部配置格式。
- 不承诺一次性删除 `enterPlanMode` / `exitPlanMode` / `upsertSessionPlan`。
- 不为当前 workflow 规模引入额外索引化结构或缓存层。
- 不修改 `session-runtime` 的 truth 边界，不让它接管 workflow 业务编排。

## Decisions

### 决策 1：将本次工作拆成“两条主线 + 两个支撑项”，而不是串行五阶段

选择：

- 主线 A：补齐 `GovernanceModeSpec` 的表达能力
- 主线 B：显式化 `compile / bind` 边界
- 支撑项 C：为 workflow 引入轻量 validate/compile 语义
- 支撑项 D：收束 prompt 来源与高频 metadata

原因：

- 当前最痛的扩展性瓶颈是 mode spec 表达力不足，而不是类型命名本身。
- 如果先做纯命名重构，再做 mode contract 扩展，很容易让 artifact / exit gate / prompt hook 继续被塞回 binder。
- 两条主线并行能保证“补 spec”与“边界收束”互相约束，而不是互相等待。

备选方案：

- 先完成一轮纯架构命名重构，再开始 spec 扩展
  - 未采纳原因：会延后对 `plan` mode 硬编码问题的处理，且新能力仍可能沿旧边界生长。

### 决策 2：`GovernanceModeSpec` 继续作为治理 DSL 核心，并扩展 mode 合同能力

选择：

- 继续围绕 `GovernanceModeSpec` 扩展，而不是新建并行的 mode contract 对象。
- 新增的表达能力应至少覆盖：
  - artifact 定义
  - exit gate
  - prompt hooks
  - workflow binding

原因：

- 插件 mode 已通过协议层直接声明 `GovernanceModeSpec`，如果再引入平行 DSL，会扩大 host/plugin 双边复杂度。
- `plan` mode 的特殊性，本质上是 mode 合同表达不够，而不是缺少另一个专用系统。
- 复用现有 mode catalog、selector 编译和 policy 编译路径，改动面更可控。

备选方案：

- 保持 `GovernanceModeSpec` 不变，把 artifact / exit gate 继续塞进 builtin tool 或 workflow 逻辑
  - 未采纳原因：这会继续固化 `plan` mode 的专有硬编码，插件仍无法定义完整 mode。

### 决策 3：治理链路保持“compile 产物”和“bound surface”两层，但不强制引入公开 normalize 类型

选择：

- 明确保留两层产物：
  - 编译产物：`CompiledModeSurface`（命名可渐进演化）
  - 绑定产物：`ResolvedGovernanceSurface`
- 不把 `NormalizedModeSpec` 作为当前阶段必须公开落地的类型。

原因：

- 现有 `GovernanceModeSpec::validate()` 已覆盖基础校验，短期不需要为了“层次完整”额外制造公开中间类型。
- 当前最重要的是把 selector 解析、policy 派生、router subset 生成视为 compiler 责任，把 runtime/profile/session/control 合并视为 binder 责任。

备选方案：

- 立即新增公开 `NormalizedModeSpec`
  - 未采纳原因：目前收益不足，且会增加额外概念负担。

### 决策 4：prompt 不新增平行 IR，继续以 `PromptPlan` 作为结果模型

选择：

- 治理层负责“决定要注入哪些 prompt”
- `adapter-prompt` 继续负责“如何渲染并产出 `PromptPlan`”
- 不再引入新的 `CompiledPromptSet`

原因：

- `PromptPlan`、`PromptBlock`、`BlockMetadata` 已经覆盖排序、来源、层级、渲染目标等职责。
- 当前真正缺失的是 prompt 来源语义与绑定责任，而不是结果模型。

备选方案：

- 引入新的治理侧 prompt IR，再交给 `adapter-prompt` 二次转换
  - 未采纳原因：与现有 `PromptPlan` 明显重叠，会增加平行概念。

### 决策 5：workflow 采用轻量 compiled artifact 语义，但不为现有规模引入索引化结构

选择：

- 为 `WorkflowDef` 增加 validate/compile 语义
- `WorkflowOrchestrator` 消费“已校验 / 已编译 workflow artifact”
- 当前保持 `Vec` 结构，不强制 `HashMap` 索引化

原因：

- 当前 workflow 规模很小，索引化不是瓶颈。
- 这里真正需要的是边界清晰，而不是数据结构升级。

备选方案：

- 直接引入 phase/transition 索引表
  - 未采纳原因：对当前规模是过度抽象，且会稀释本次 change 的重点。

### 决策 6：plugin reload 必须提升为治理一致性问题，而不是局部实现细节

选择：

- mode catalog、capability surface、skill catalog 的替换必须形成统一候选快照
- 成功时一起切换，失败时一起回滚
- 运行中的 turn 继续使用旧 surface；下一 turn 才使用新快照

原因：

- 当前 reload 已有“能力面失败则回滚 surface”的雏形，但 mode catalog 与 skill catalog 没有统一的一致性契约。
- plugin mode 已经是正式 DSL 输入，如果 reload 失败后 mode catalog 与 capability surface 漂移，后续编译就会得到不一致结果。

备选方案：

- 只要求 capability surface 原子替换，mode catalog/skill catalog 由调用方自行协调
  - 未采纳原因：这会把一致性责任散落到多个模块，后续难以验证。

## Risks / Trade-offs

- [风险] `GovernanceModeSpec` 扩展后，builtin mode 与 plugin mode 的校验复杂度上升
  - Mitigation：把新增字段设计为显式可选，并为 mode catalog 注册增加集中校验和错误归类。

- [风险] compile/bind 命名收束期间，新旧术语并存会让代码短期更难读
  - Mitigation：优先补模块注释和类型注释，再做渐进重命名，避免“一次性全改名”。

- [风险] `plan` mode 通用化过程中可能影响现有 `enterPlanMode` / `exitPlanMode` / `upsertSessionPlan` 行为
  - Mitigation：先让 mode spec 能表达等价合同，再逐步把 builtin plan 迁移到新合同上，保留明确回滚点。

- [风险] reload 一致性提升后，重载路径实现会更复杂
  - Mitigation：以“候选快照 + 提交/回滚”模型收敛更新步骤，并补充失败路径测试。

- [风险] workflow validate/compile 语义补入后，可能诱发额外抽象冲动
  - Mitigation：明确当前非目标是不做索引化与过度目录拆分，只补边界，不追求形式完整。

## Migration Plan

1. 先更新架构文档和相关 specs，固定 compile/bind/orchestrate 与 mode contract 术语。
2. 在 `core` 扩展 `GovernanceModeSpec` 所需字段，并补充 mode 校验逻辑。
3. 在 `application` 中把 mode compile 产物与 governance binder 的边界显式化。
4. 让 builtin `plan` mode 先以新 spec 字段表达现有语义，再视实现节奏决定是否通用化 builtin tools。
5. 为 workflow 加入轻量 validate/compile 边界，并保持当前数据结构。
6. 调整 bootstrap / reload 逻辑，保证 mode catalog、capability surface、skill catalog 的一致性切换。
7. 补充 selector 编译、plan mode 合同、plugin reload 回滚、workflow compile 与 prompt 来源的测试。

回滚策略：

- 若 mode spec 扩展或 reload 一致性改造引发不稳定，可保留新的 spec 字段但继续由 builtin plan 走旧逻辑。
- 若 compile/bind 重命名带来阅读或迁移成本过高，可先保留旧类型名，通过注释与包装函数明确语义，待后续 change 再逐步改名。

## Open Questions

- mode 级 artifact 合同是否只覆盖单 artifact，还是需要从一开始支持多 artifact 及命名槽位？
- exit gate 应定义为通用规则表达式，还是先收敛成少量内建 gate 类型？
- workflow binding 应落在 `GovernanceModeSpec` 内，还是由 workflow spec 引用 mode contract 并做双向校验？
- reload 的"一致性提交"最终应由 `AppGovernance`、`ServerRuntimeReloader` 还是更底层的组合根对象统一承载？


## Resolved Questions

- **单 artifact vs 多 artifact**：本次只支持单 artifact。当前 plan mode 只有 1 个 artifact，多 artifact 需求不明确，等有真实场景再扩展。
- **exit gate 形状**：先收敛为内建 gate 类型（`required_headings` + `actionable_sections` + `review_passes` + `review_checklist`）。不引入通用规则表达式。
- **workflow binding 位置**：放在 `GovernanceModeSpec` 内。插件声明 mode 时应能同时声明它属于哪个 workflow phase，这比让 workflow spec 反向引用 mode 更简单。
- **reload 一致性承载方**：由 `AppGovernance` 统一承载。它已经是治理组合根，mode catalog / capability surface / skill catalog 的候选快照提交/回滚应由它协调。
