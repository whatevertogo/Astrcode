## Context

Astrcode 现有实现已经提供了 4 个和本次设计直接相关的事实：

1. `WorkflowExamplesContributor` 已经承担了共享协作协议和 few-shot 示例，但它位于 dynamic layer，适合放“通用规则”，不适合承载稳定的 delegation 目录。
2. `AgentProfileSummaryContributor` 已经是一个半稳定 prompt contributor，负责把 child-eligible profile 列进 prompt，但现在仍然只是简单的 `id + description` 列表。
3. `PromptDeclarationContributor` 已经承担 child inherited prompt block 的注入与分层，是当前最自然的 child-specific contract 注入边界。
4. `ObserveAgentResult` 已经明确区分原始事实和 advisory projection，说明结果投影层已经有合适落点，不需要另造平行真相。

这几条意味着：Astrcode 不需要为了“更像 Claude Code”而发明新的巨大 runtime 概念，真正需要的是把现有 prompt contributor、prompt declaration 和 collaboration result 三条链路收口成一套更清晰的 delegation surface。

同时必须注意一条项目约束：`AgentProfile` 在本仓库已经被明确收敛成行为模板，真正的 capability truth 来自 parent surface、spawn grant 和 runtime availability 的交集。因此 Astrcode 不能照搬 Claude Code 那种“agent 列表自带工具权限摘要”的设计，否则会重新把 profile 拉回权限真相。

## Goals / Non-Goals

**Goals:**

- 基于现有 contributor 架构定义一套正式的 delegation surface，而不是继续把所有协作知识塞进工具 description。
- 让 `spawn` guidance 明确区分 fresh child、resumed child、restricted child 三种 mode，并与现有 runtime 语义对齐。
- 让行为模板目录只表达“什么样的 child 适合做什么事”，而不是伪装成 capability 授权目录。
- 让 `observe` / child result projection 更容易表达“继续复用这个 child、关闭这个分支，还是另起一个更合适的 child”。
- 保持现有 capability truth、child session 持久化模型和 direct-parent 所有权边界不变。

**Non-Goals:**

- 不改当前 `spawn / send / observe / close` 的基本工具拓扑。
- 不引入 background child、remote worker、teammate network 或 worktree isolation。
- 不把新的 delegation metadata 变成另一套 durable 业务真相；原始 lifecycle、turn outcome、resolved capability surface 仍然是事实源。
- 不让 profile 目录重新承担工具授权表达职责。
- 不在本 change 中处理 workflow preset 的产品化或前端完整任务面板。

## Decisions

### 决策 1：采用“三层 delegation prompt 架构”，并直接落在现有 contributor 边界上

这次不引入全新的 prompt 体系，而是在现有架构上明确三层分工：

- 共享协议层：放在 `WorkflowExamplesContributor`，负责四工具的一般决策协议和 mode-level guidance。
- 行为模板目录层：放在 `AgentProfileSummaryContributor` 或其等价增强版本，负责展示当前可委派的 child behavior template。
- child 专属合同层：放在 `PromptDeclarationContributor` 这条 inherited 注入路径，负责某个 child launch / resume 时的执行合同。

这样做的原因：

- `workflow_examples` 已经位于 dynamic layer，适合共享协作协议，但不适合承载稳定目录。
- `agent_profile_summary` 已经位于 semi-stable layer，天然适合承载当前 working-dir 下的行为模板目录。
- `prompt_declaration` 已经有 child inherited block 的现成模式，最适合承接 child-specific contract。

替代方案一是把 catalog、child contract 和 mode guidance 全塞进 `spawn` description；会让工具说明膨胀成混合层。  
替代方案二是新建一个大而全 contributor 承接全部内容；会绕开现有分层，而不是复用它。

### 决策 2：行为模板目录只表达“适合做什么”，不表达“拥有什么工具”

Astrcode 当前的架构约束已经很清楚：

- `AgentProfile` 表示行为模板
- `capabilityGrant` 表示这次任务申请的最小能力子集
- `resolved capability surface` 才是 child launch-time 的执行真相

因此行为模板目录里不能仿照 Claude Code 去展示“这个 profile 带哪些工具”。更合适的目录内容是：

- `type`
- 行为模板摘要 / when-to-use
- 默认协作风格或使用建议

至于 restricted child 的 capability-aware 信息，应该在 child 启动后的 execution contract 里出现，而不是伪装成 profile 固有属性。

替代方案是给每个 profile 做一个“工具摘要”，但这会把 profile 再次变成权限组合表，直接违背刚建立的 capability 设计。

### 决策 3：把 delegation mode 视为一等 prompt 语义

本次把 child delegation 分成三类：

- fresh child：第一次承担一个新的、隔离的责任边界
- resumed child：复用既有 responsibility continuity，只追加下一条具体指令
- restricted child：launch-time capability surface 被显式收缩，不能承担超出该 surface 的工作

这些 mode 不一定都要体现在外部 DTO 上，但必须体现在 prompt 组织和 tool guidance 上。这样模型才能区分：

- fresh child 需要完整 briefing
- resumed child 需要最小 delta instruction
- restricted child 需要 capability-aware task assignment

替代方案是保持现在“统一一套 spawn guidance”不变，只靠模型自己理解上下文。这个方案会持续制造 briefing 过宽、重复解释和 capability mismatch 的低效。

### 决策 4：child execution contract 通过 child-scoped prompt declaration 注入

这条合同不应继续依赖 `spawn.prompt` 自然语言自行约定，而应通过 child-scoped prompt surface 明确提供。最适合的落点是当前已经存在的 inherited / prompt-declaration 链路。

合同内容至少包括：

- 当前责任边界
- 期待的回传方式
- 如果是 restricted child，则明确说明 launch-time capability limit 的摘要

其中：

- fresh child：需要完整责任 + 交付 + 限制
- resumed child：保留原责任边界，只补 delta instruction
- restricted child：明确说明 capability-aware 限制，避免 child 再向外扩写职责

替代方案是只在 `spawn.prompt` 里塞更多样板文案；这太依赖调用方 prompt 质量，不利于长期稳定复用。

### 决策 5：共享协作协议继续留在 `workflow_examples`，工具 description 保持短而硬

`WorkflowExamplesContributor` 目前已经承载：

- few-shot 行为示例
- child collaboration guide

这意味着它非常适合继续承载：

- `Idle` 是正常可复用状态
- `spawn / send / observe / close` 的统一决策协议
- fresh / resume / restricted 三种 mode 的高层原则

但不适合再继续膨胀成目录或 child contract 容器。  
与此同时，`spawn_tool.rs` / `send_tool.rs` / `observe_tool.rs` 的 description 和 prompt metadata 应继续保持动作导向，只说明“这一步工具本身怎么用”。

### 决策 6：结果投影继续是 advisory，但要补足“责任连续性”与“mismatch 线索”

`ObserveAgentResult` 现有结构已经提供了一个很好的边界：原始状态字段 + advisory decision fields。  
这次继续沿用这个边界，但补足两类目前不够稳定的线索：

- `recommendedNextAction`
- `recommendedReason`
- responsibility continuity
- capability mismatch 或 responsibility mismatch 的原因提示

但这些字段必须保持 advisory。真正的事实源仍然是：

- child lifecycle
- last turn outcome
- pending message 状态
- resolved capability surface

这样既能给模型足够强的下一步线索，又不会破坏“Server is the truth”的边界。

替代方案是把这些新字段写成 durable 状态真相或单独 DTO 主事实源。这会让 projection 反客为主，后续维护成本更高。

## Risks / Trade-offs

- [Risk] prompt surface 变多，反而增加上下文负担  
  → Mitigation：把共享协议、行为模板目录、child 专属合同严格分层，并限制目录只展示必要的行为模板信息。

- [Risk] 行为模板目录被误解成 capability 授权目录  
  → Mitigation：目录中不展示伪造的 per-profile tool list；restricted capability 信息只在 child contract 与结果投影中出现。

- [Risk] fresh / resume / restricted 三种 mode 的边界在实现上不够稳定  
  → Mitigation：优先围绕当前已有的 spawn、resume、grant 路径做判定，不额外发明复杂状态机。

- [Risk] 结果投影过强，模型把建议动作当成强制命令  
  → Mitigation：保留原始事实字段并在文案中显式标注 advisory 语义。

- [Risk] 以后若做前端任务面板，当前 contract 可能需要再抽象一次  
  → Mitigation：这次先稳定“模型可见 contract”和“工具结果 projection”，避免过早承诺前端产品形态。

## Migration Plan

1. 先增强 `workflow_examples`、`agent_profile_summary` 和 `prompt_declaration` 三条现有 contributor 路径，形成明确分工。
2. 收紧 `spawn / send / observe / close` 的工具 description，使其只保留动作级 guidance，不再尝试同时承担目录和 child contract。
3. 在 `application` / `session-runtime` 的 child launch、resume、observe 路径上补齐 delegation metadata projection，供 child contract 和 tool result 共用。
4. 如果现有 HTTP / 前端调试面需要复用这些 projection，再以只读方式向外暴露；否则第一阶段先限定在 prompt / tool-facing 结果。
5. 回滚时优先撤回新增的 child contract 与 advisory projection，不动现有 capability truth、profile 解析和 child lifecycle 主链路。

## Open Questions

- restricted child 的 capability mismatch 提示是否需要结构化字段，还是先停留在 `recommendedReason` 的语义里即可？
- 行为模板目录是否需要扩展 `PromptAgentProfileSummary` 结构，还是现阶段继续依赖 `description` 的精简摘要足够？
- 如果后续引入 background child，这套 delegation contract 哪些字段可以直接复用，哪些需要重新定义？
