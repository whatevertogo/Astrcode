## ADDED Requirements

### Requirement: authoritative collaboration guidance SHALL be assembled outside adapter-owned static prompt code

协作 guidance 的 authoritative 来源 MUST 来自统一治理装配路径，而不是继续散落在 adapter 层的静态 builtin prompt 代码中。

#### Scenario: adapter renders but does not own collaboration truth

- **WHEN** 模型 prompt 中出现协作 guidance
- **THEN** `adapter-prompt` SHALL 只负责渲染该 guidance 对应的 `PromptDeclaration`
- **AND** SHALL NOT 继续把协作治理真相直接硬编码在 contributor 内作为唯一事实源

#### Scenario: multiple entrypoints receive consistent collaboration guidance

- **WHEN** root execution、普通 session submit 与 child execution 都需要协作 guidance
- **THEN** 它们 SHALL 从同一治理装配路径获得一致的协作声明
- **AND** SHALL NOT 因入口不同而依赖不同的硬编码文本来源
