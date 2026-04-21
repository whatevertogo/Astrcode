## MODIFIED Requirements

### Requirement: workflow orchestration 与 HookHandler 系统保持分层边界

生命周期 hooks 平台与 workflow orchestrator MUST 保持分层边界。hooks 平台可以在 turn、tool、permission、compact、subagent 等生命周期点观察和补充上下文，也可以基于已解析的 workflow truth 产出 prompt overlays；但 workflow phase 的 signal 解释、transition 裁决、bridge truth 与持久化仍 MUST 归属于 workflow orchestration。hooks SHALL NOT 直接决定 phase 迁移真相。

#### Scenario: workflow orchestration 不直接消费 hook 返回值

- **WHEN** workflow orchestrator 解释用户输入、phase bridge 或审批信号
- **THEN** 它 SHALL 只依赖 workflow state、session facts 与 typed workflow signals
- **AND** SHALL NOT 直接读取 hooks 平台的任意返回值来决定 phase 迁移真相

#### Scenario: hooks can observe workflow-resolved context without owning transitions

- **WHEN** `BeforeTurnSubmit` hook 处理一个已解析出 active workflow phase 的 turn
- **THEN** 它 MAY 基于当前 `phase_id`、artifact refs 或 bridge payload 产出 prompt overlay
- **AND** SHALL NOT 自行解释自由文本来决定 approval、replan 或 phase 切换

## ADDED Requirements

### Requirement: workflow-specific overlays SHALL be delivered through lifecycle hook effects on top of resolved phase truth

workflow phase 的 prompt overlays MUST 建立在已解析完成的 phase truth 与 bridge context 之上，并通过 hooks 平台的 turn-level effect 进入 prompt 组装，而不是继续散落在 plan-specific helper 或提交流程分支里。

#### Scenario: executing phase bridge overlay is emitted after phase truth is resolved

- **WHEN** session 已处于 `executing` phase，且 bridge state 包含 approved plan artifact 与步骤摘要
- **THEN** 系统 SHALL 允许 builtin `BeforeTurnSubmit` hook 基于该 bridge truth 产出 execute overlay
- **AND** SHALL NOT 要求主提交流程直接手工拼接这类 declaration

#### Scenario: invalid workflow state fallback happens before hook overlay generation

- **WHEN** workflow state 文件损坏或语义无效，系统降级到 mode-only 路径
- **THEN** hooks 平台 SHALL 只接收降级后的有效 turn context
- **AND** SHALL NOT 让 workflow overlay hooks 自己决定恢复策略
