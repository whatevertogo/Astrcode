## ADDED Requirements

### Requirement: prompt declaration visibility filtering SHALL be driven by the governance envelope, not implicit capability name matching

`prompt_declaration_is_visible`（server/bootstrap/prompt_facts.rs:200-213）当前通过 `allowed_capability_names` 过滤 prompt declaration 的可见性。这个联动 MUST 变为显式的治理包络驱动，而不是通过隐式的字符串集合匹配。

#### Scenario: declaration visibility uses governance-resolved capability surface

- **WHEN** `PromptFactsProvider` 需要决定哪些 prompt declaration 对当前 turn 可见
- **THEN** 过滤逻辑 SHALL 使用治理包络中已解析的 capability surface
- **AND** SHALL NOT 独立从 `ResolvedExecutionLimitsSnapshot.allowed_tools` 重建过滤集合

#### Scenario: visibility filtering is consistent across prompt facts and turn execution

- **WHEN** turn 执行链路使用治理包络中的 capability router 决定工具可见性
- **THEN** prompt facts 的 declaration 过滤 SHALL 使用同一能力面事实源
- **AND** SHALL NOT 出现工具可见但 declaration 被过滤（或反之）的不一致

### Requirement: PromptFacts metadata governance parameters SHALL come from the governance envelope

`PromptFacts.metadata` 当前通过 vars dict 注入 `agentMaxSubrunDepth` 和 `agentMaxSpawnPerTurn` 等治理参数。这些参数 MUST 从治理包络中显式获取，而不是通过松散的 string-keyed dict 传递。

#### Scenario: agent limits in prompt facts come from the envelope

- **WHEN** `resolve_prompt_facts` 构建 `PromptFacts.metadata`
- **THEN** `agentMaxSubrunDepth` 和 `agentMaxSpawnPerTurn` SHALL 从治理包络中读取
- **AND** SHALL NOT 从 `ResolvedAgentConfig` 独立读取并通过 vars dict 注入

#### Scenario: metadata keys are strongly typed through the governance path

- **WHEN** 治理参数通过治理包络传递到 prompt facts
- **THEN** 参数传递 SHALL 使用结构化类型，而不是 string-keyed hashmap
- **AND** SHALL 减少因 key 名拼写错误或类型不匹配导致的隐式失败

### Requirement: profile context governance fields SHALL come from the governance envelope

`build_profile_context`（prompt_facts.rs:107-136）当前注入 `approvalMode`、`sessionId`、`turnId` 等治理上下文字段。这些字段 MUST 与治理包络中的信息保持一致。

#### Scenario: approval mode in profile context aligns with envelope

- **WHEN** `build_profile_context` 注入 `approvalMode`
- **THEN** approvalMode 的值 SHALL 与治理包络中的策略引擎配置一致
- **AND** SHALL NOT 出现 profile context 中的 approvalMode 与实际策略引擎行为不一致的情况

#### Scenario: session and turn identifiers in profile context come from the governance path

- **WHEN** profile context 包含 sessionId 和 turnId
- **THEN** 这些标识符 SHALL 与治理包络中记录的标识符一致
- **AND** SHALL NOT 从独立的参数源重新获取

### Requirement: PromptFactsProvider SHALL be a consumer of the governance surface, not an independent governance assembler

`PromptFactsProvider` 当前同时承担"收集 prompt 事实"和"做隐式治理过滤"两个职责。cleanup 后，它 MUST 只负责收集和渲染事实，治理过滤逻辑 MUST 上移到治理装配层。

#### Scenario: PromptFactsProvider delegates governance filtering to the assembler

- **WHEN** `resolve_prompt_facts` 执行
- **THEN** 它 SHALL 接收治理装配器已过滤的 prompt declarations 和 capability surface
- **AND** SHALL NOT 自行实现 `prompt_declaration_is_visible` 过滤逻辑
