## ADDED Requirements

### Requirement: turn-level hooks SHALL execute as part of governance surface assembly before runtime submission

所有 turn 入口在把请求提交给 `session-runtime` 之前，MUST 经过 hooks 平台的 turn-level 事件解析，至少覆盖 `BeforeTurnSubmit`。治理装配层 SHALL 负责把合法 hook effects 合并进最终治理包络，并确保不同入口的行为一致。

#### Scenario: session prompt submission runs before-turn hooks

- **WHEN** 普通 session prompt 提交即将进入治理装配
- **THEN** 系统 SHALL 先触发 `BeforeTurnSubmit` hooks
- **AND** 再把 hook 产出的合法 prompt declarations / system messages 合并到本次治理包络

#### Scenario: root and subagent entrypoints use the same hook path

- **WHEN** root execution、fresh child launch 或 resumed child submit 触发 turn 提交
- **THEN** 它们 SHALL 通过同一 turn-level hook 解析路径进入治理装配
- **AND** SHALL NOT 因入口不同而绕开 hooks 平台

### Requirement: governance surface SHALL validate hook effects against existing governance boundaries

治理装配层在消费 hook effects 时 MUST 以 capability surface、policy verdict、execution limits 和 prompt injection path 作为硬边界。装配层 SHALL 只接受与当前事件类型匹配且未突破治理边界的 effect。

#### Scenario: hook prompt additions still use PromptDeclaration path

- **WHEN** `BeforeTurnSubmit` hook 为当前 turn 追加 prompt 相关 effect
- **THEN** 装配层 SHALL 将其转化为 `PromptDeclaration`
- **AND** SHALL 继续通过现有 prompt declaration 注入路径进入 prompt 组装

#### Scenario: invalid permission-widening effect is rejected

- **WHEN** 某个 hook effect 试图扩大当前 turn 已解析完成的允许工具面或越过 policy hard deny
- **THEN** 装配层 SHALL 拒绝该 effect
- **AND** SHALL 记录诊断信息而不是静默接受
