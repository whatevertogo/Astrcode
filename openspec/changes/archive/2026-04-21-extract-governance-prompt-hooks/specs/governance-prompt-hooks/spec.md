## ADDED Requirements

### Requirement: governance prompt hooks SHALL resolve turn-scoped PromptDeclarations from typed submission contexts

系统 MUST 在 `application` 层提供 governance prompt hook 解析能力，用于在 turn 提交前根据 typed submission context 生成额外的 `PromptDeclaration`。该解析能力 SHALL 面向 mode、artifact 与 workflow 上下文，而不是直接面向工具调用或 compact 生命周期。

#### Scenario: planning context emits plan facts and template guidance

- **WHEN** 当前 session 处于 `plan` mode，且本次提交没有 active plan artifact
- **THEN** governance prompt hooks SHALL 生成描述 target plan 路径/slug 的 facts declaration
- **AND** SHALL 额外生成首次规划所需的 template guidance declaration

#### Scenario: executing workflow context emits bridge guidance

- **WHEN** 当前 session 的 active workflow 处于 `executing` phase，且 bridge state 已包含 approved plan 引用与 implementation steps
- **THEN** governance prompt hooks SHALL 生成 execute bridge declaration
- **AND** 该 declaration SHALL 包含 approved plan artifact 引用与步骤摘要

### Requirement: governance prompt hooks SHALL execute deterministically and compose contributions through one resolver

系统 MUST 通过统一 resolver 执行 governance prompt hooks。resolver SHALL 使用稳定顺序解析匹配的 hooks，并把产出的 declarations 组合成单一结果集合，供后续治理装配使用。

#### Scenario: matching hooks run in stable order

- **WHEN** 多个 governance prompt hooks 同时匹配同一 submission context
- **THEN** resolver SHALL 按稳定注册顺序产出 declarations
- **AND** 同一输入重复解析时 SHALL 得到等价的 declaration 顺序

#### Scenario: non-matching hooks stay silent

- **WHEN** 某个 governance prompt hook 不匹配当前 submission context
- **THEN** resolver SHALL 不为该 hook 产出 declaration
- **AND** SHALL NOT 因未匹配而阻塞其他 hook 的解析

### Requirement: governance prompt hooks SHALL consume orchestration-prepared facts and MUST NOT own persistence truth

governance prompt hooks MUST 只消费 orchestration 预先装配好的 typed facts。hook 本身 SHALL NOT 负责 session/workflow 持久化读取、phase 迁移、mode 切换或 durable event 写入。

#### Scenario: hooks consume preloaded plan facts instead of reopening state files

- **WHEN** governance prompt hooks 需要基于 plan artifact 状态生成 declarations
- **THEN** hook 输入 SHALL 已包含所需的 plan summary / prompt context
- **AND** hook 自身 SHALL NOT 再独立读取 `state.json` 或 plan markdown 文件来决定是否匹配

#### Scenario: workflow downgrade happens before hook resolution

- **WHEN** orchestration 发现 workflow state 文件损坏或语义无效
- **THEN** 系统 SHALL 先按既有恢复策略降级到 mode-only 路径
- **AND** governance prompt hooks SHALL 只接收降级后的有效 submission context，而不是自己决定恢复策略

### Requirement: governance prompt hooks SHALL remain separate from lifecycle HookHandler

`governance prompt hooks` 与 `core::hook::HookHandler` MUST 是两套分层独立的机制。前者面向 turn-scoped prompt declaration 解析，后者面向工具调用与 compact 生命周期扩展；两者 SHALL NOT 复用同一事件枚举或同一执行责任。

#### Scenario: prompt hook resolution does not trigger lifecycle hook events

- **WHEN** 系统在 turn 提交前解析 governance prompt hooks
- **THEN** 它 SHALL NOT 触发 `PreToolUse`、`PostToolUse`、`PreCompact` 或 `PostCompact` 事件
- **AND** SHALL NOT 要求把 prompt overlay 包装为 `HookHandler`

#### Scenario: lifecycle hooks do not decide prompt overlay content

- **WHEN** 插件 `HookHandler` 处理工具调用或 compact 事件
- **THEN** 它 SHALL 继续只作用于该生命周期节点
- **AND** SHALL NOT 直接决定本次 turn 的 mode/workflow prompt overlay 内容
