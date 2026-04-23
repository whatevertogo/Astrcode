## Purpose

定义正式 workflow 的 phase 图、迁移协议、bridge 边界与恢复策略，作为 `application` 层 workflow orchestration 的主规范。

## Requirements

### Requirement: 正式 workflow SHALL 由可组合的 phase 图驱动

系统 MUST 把正式 workflow 定义为一组 `phase`、`transition` 与 `bridge`，而不是把完整流程硬编码在某个 mode、tool 或单一提交入口中。每个 phase SHALL 至少声明：稳定 `phase id`、绑定的 `mode id`、phase role、可选 artifact 规则，以及允许触发的 transition。

#### Scenario: `plan_execute` workflow 定义 planning 与 executing 两个 phase

- **WHEN** 系统装载内建的 `plan_execute` workflow
- **THEN** 它 SHALL 至少包含 `planning` phase（绑定 `plan` mode）与 `executing` phase（绑定 `code` mode）
- **AND** phase 定义 SHALL 明确各自的 role、允许迁移的目标和所依赖的 bridge

#### Scenario: phase 复用治理 mode 而不重建 mode catalog

- **WHEN** 一个 workflow phase 绑定到某个既有 `mode id`
- **THEN** 系统 SHALL 复用该 mode 已有的 governance envelope 编译结果
- **AND** SHALL NOT 为该 phase 重新定义平行的 mode catalog 或 capability router 真相

### Requirement: workflow 协议 SHALL 显式定义 transition、signal 与 bridge state 的结构

workflow 协议 MUST 为 transition、signal 与 bridge state 提供稳定字段，而不是只在设计中以名称引用。transition、signal 与 bridge state 都必须可序列化、可测试、可在恢复时重建。

#### Scenario: transition 定义明确来源、目标与触发器

- **WHEN** 系统声明一个 `WorkflowTransitionDef`
- **THEN** 它 SHALL 至少包含 `transition_id`、`from_phase_id`、`to_phase_id` 与 typed `trigger`
- **AND** `trigger` SHALL 明确区分 `Signal`、`Auto` 与 `Manual` 类触发

#### Scenario: signal 进入 orchestration 前被收敛为 typed enum

- **WHEN** 用户自由文本或工具结果被解释为 workflow 信号
- **THEN** 进入 orchestrator 的信号 SHALL 已收敛为 typed `WorkflowSignal`
- **AND** SHALL NOT 让自由字符串直接决定 phase 迁移

#### Scenario: bridge state 使用稳定 envelope 持久化

- **WHEN** workflow phase 迁移需要跨 phase 传递桥接上下文
- **THEN** 系统 SHALL 使用稳定的 `WorkflowBridgeState` envelope 持久化 `bridge_kind`、源/目标 phase、版本与 payload
- **AND** 具体 bridge 的 typed payload MAY 由 `application` 层定义并序列化到 envelope 的 payload 中

### Requirement: 每个 session SHALL 维护显式的 active workflow instance

当 session 进入正式 workflow 后，系统 MUST 持久化 active workflow instance，至少记录 `workflow_id`、`current_phase_id`、phase-owned artifact 引用与最近更新时间。active workflow instance MUST 是显式持久化状态，而不是只存在内存中的隐式分支。

#### Scenario: session reload 后恢复 active workflow phase

- **WHEN** 一个带有 active workflow instance 的 session 被重新加载
- **THEN** 系统 SHALL 恢复该 workflow 的 `current_phase_id` 与关联 artifact 引用
- **AND** 下一次提交 SHALL 继续按恢复后的 phase 解释用户输入与 prompt overlay

#### Scenario: 没有 active workflow 的 session 继续按普通 mode 运行

- **WHEN** 当前 session 不存在 active workflow instance
- **THEN** 系统 SHALL 继续按现有 mode/governance 提交流程运行
- **AND** SHALL NOT 因引入 workflow 系统而要求所有 session 都绑定一个 workflow

#### Scenario: workflow state 恢复失败时降级为 mode-only 路径

- **WHEN** workflow instance state 文件缺失或损坏
- **THEN** 系统 SHALL 将该 session 视为没有 active workflow
- **AND** SHALL 继续按现有 mode-only 路径运行
- **AND** SHALL 记录一条包含损坏路径的警告日志

### Requirement: workflow 恢复 SHALL 独立于 session-runtime recovery checkpoint

workflow instance state 与 `SessionRecoveryCheckpoint` 是两套不同职责的持久化状态：前者记录 workflow/phase truth，后者记录 session-runtime 的投影与恢复快照。两者 MUST 独立恢复，且 workflow state 损坏不得阻塞 session-runtime 的恢复。

#### Scenario: session-runtime recovery 先于 workflow recovery

- **WHEN** 系统重新加载一个 session
- **THEN** 它 SHALL 先恢复 session-runtime 的 checkpoint 与 tail events
- **AND** 仅在 runtime 恢复完成后，再由 `application` 尝试加载 workflow instance state

#### Scenario: checkpoint 与 workflow state 失败策略彼此独立

- **WHEN** workflow state 文件损坏但 runtime checkpoint 正常
- **THEN** session SHALL 继续恢复成功，并降级到 mode-only 路径
- **AND** SHALL NOT 因 workflow state 损坏而阻塞整个 session 加载

### Requirement: workflow orchestration SHALL 在提交边界解释用户信号并驱动 phase 迁移

系统 MUST 在 turn 提交边界解释用户消息与已知 workflow 信号，并据此决定：保持当前 phase、迁移到下一 phase、或回退到上一个 phase。该逻辑 SHALL 归属于 workflow orchestration，而不是散落在 plan-specific if/else 或 prompt 暗示中。

#### Scenario: planning phase 中的 approval 信号推进到 executing

- **WHEN** 当前 active workflow 处于 `planning` phase，且用户消息匹配该 phase 的 approval 规则
- **THEN** 系统 SHALL 把 active workflow 迁移到 `executing` phase
- **AND** SHALL 通过统一 mode 切换入口把 session 切换到 `code` mode

#### Scenario: executing phase 中的 replan 信号回退到 planning

- **WHEN** 当前 active workflow 处于 `executing` phase，且用户显式触发 `replan` 类信号
- **THEN** 系统 SHALL 把 active workflow 迁移回 `planning` phase
- **AND** 下一次提交 SHALL 恢复 planning phase 的 overlay 与 artifact 规则，而不是继续沿用 execute guidance

### Requirement: phase bridge SHALL 传递 artifact 上下文而不合并 durable truth

phase 之间的 bridge MUST 把 source phase 的关键 artifact 上下文转换为 target phase 可消费的输入，但 SHALL NOT 直接把两边的 durable truth 合并成同一份状态。bridge 输出可以是 prompt overlay、artifact reference 或结构化 bridge state。

#### Scenario: approved plan 进入 executing phase 时生成 execute bridge context

- **WHEN** `planning` phase 的 canonical plan 已批准并触发进入 `executing` phase
- **THEN** 系统 SHALL 为 `executing` phase 生成显式 bridge context，其中至少包含 approved plan 引用和可执行步骤摘要
- **AND** execute phase SHALL 通过该 bridge context 理解 plan->execute handoff，而不是只依赖自由文本提示

#### Scenario: bridge 不直接写入 execution task durable snapshot

- **WHEN** phase bridge 把 approved plan 交接给 execute phase
- **THEN** 系统 SHALL NOT 自动生成或覆盖 `taskWrite` durable snapshot
- **AND** execution task truth 仍 SHALL 只由 task 系统自己的写入口维护

### Requirement: workflow phase 迁移 SHALL 定义明确的持久化边界与失败策略

workflow phase 迁移涉及 signal 解释、bridge 计算、workflow state 文件写入、mode 切换和 overlay 生成。系统 MUST 明确哪一步是主记录、失败时如何补偿，而不是依赖隐式成功顺序。

#### Scenario: workflow state 是 phase 迁移的主记录

- **WHEN** 系统准备从一个 phase 迁移到另一个 phase
- **THEN** 它 SHALL 先验证 transition 和 bridge
- **AND** SHALL 先原子写入新的 `WorkflowInstanceState`
- **AND** 再通过统一 mode 切换入口写入 `ModeChanged` durable event

#### Scenario: mode 切换失败后通过 phase->mode 关系补偿

- **WHEN** workflow state 已成功写入目标 phase，但 mode 切换失败
- **THEN** 系统 SHALL 保留新的 workflow phase
- **AND** SHALL 在下一次提交或恢复时按 `current_phase_id -> mode_id` 做 reconcile
- **AND** SHALL NOT 试图从 mode 反推 workflow phase，因为同一 mode MAY 被多个 phase 复用

### Requirement: 第一阶段 workflow state 对前端 SHALL 保持内部可见

第一阶段 workflow state MUST 主要作为 application 内部状态使用。系统 SHALL NOT 在本 change 中引入新的 workflow phase durable event、前端 workflow 面板或对外 query surface，除非为了兼容既有能力必须暴露最小事实。

#### Scenario: workflow phase 变化不新增前端 durable event

- **WHEN** active workflow 发生 phase 迁移
- **THEN** 系统 SHALL 更新内部 workflow instance state
- **AND** SHALL NOT 在本 change 中额外写入 `WorkflowPhaseChanged` 类 durable event
- **AND** 前端继续通过既有 mode / transcript / task surface 工作

### Requirement: workflow orchestration 与 HookHandler 系统保持分层边界

`core::hook` 的 `HookHandler`（`PreToolUse` / `PostToolUse` / `PreCompact` / `PostCompact`）粒度为单次工具调用或压缩，面向插件扩展。workflow orchestrator 粒度为 turn 提交边界，面向业务编排。两者 SHALL NOT 在同一层竞争：hook 不感知 workflow phase，workflow 不直接消费 hook 结果。

#### Scenario: workflow orchestration 不直接消费 hook 返回值

- **WHEN** workflow orchestrator 解释用户输入、phase bridge 或审批信号
- **THEN** 它 SHALL 只依赖 workflow state、session facts 与 typed workflow signals
- **AND** SHALL NOT 直接读取 `HookHandler` 的执行结果来决定 phase 迁移

#### Scenario: HookHandler 不感知 workflow phase

- **WHEN** 一个 `HookHandler` 处理 `PreToolUse`、`PostToolUse`、`PreCompact` 或 `PostCompact`
- **THEN** 它 SHALL 继续只处理该 hook 自身的工具或压缩语义
- **AND** SHALL NOT 读取、修改或推断 active workflow phase
