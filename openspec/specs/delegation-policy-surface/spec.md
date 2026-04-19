## Purpose

定义 delegation 策略相关参数（DelegationMetadata、SpawnCapabilityGrant、AgentCollaborationPolicyContext、spawn budget）如何从治理包络统一获取，消除分散读取。

## Requirements

### Requirement: delegation metadata SHALL be generated from the unified governance assembly path

`DelegationMetadata`（responsibility_summary、reuse_scope_summary、restricted、capability_limit_summary）MUST 由统一治理装配器生成。`build_delegation_metadata`（`governance_surface/prompt.rs`）作为装配器的内部 helper，由 `GovernanceSurfaceAssembler` 的 `fresh_child_surface` 和 `resumed_child_surface` 统一调用。

#### Scenario: delegation metadata comes from the governance assembler

- **WHEN** 系统启动或恢复一个 child session
- **THEN** `DelegationMetadata` SHALL 由治理装配器根据治理包络中的 child policy 统一生成
- **AND** `build_delegation_metadata` 只通过治理装配路径调用，不从外部独立调用

#### Scenario: delegation metadata is consistent across fresh and resumed child

- **WHEN** fresh child 和 resumed child 各自生成 delegation metadata
- **THEN** 两者的 metadata 字段含义和来源 SHALL 一致
- **AND** SHALL NOT 因 fresh/resumed 路径不同而使用不同的 metadata 生成逻辑

### Requirement: SpawnCapabilityGrant SHALL be resolved from the governance envelope, not passed as ad-hoc spawn parameters

`SpawnCapabilityGrant` 当前作为 `SpawnAgentParams` 的字段由调用方直接构造。它 MUST 从治理包络中解析，使 child 的能力授权受统一治理决策约束。

#### Scenario: capability grant comes from governance-resolved child policy

- **WHEN** 系统确定一个 child 允许使用的工具集合
- **THEN** `SpawnCapabilityGrant.allowed_tools` SHALL 由治理装配器根据 child policy 与 parent capability surface 计算得出
- **AND** SHALL NOT 由 spawn 调用方从模型参数中直接构造

### Requirement: AgentCollaborationPolicyContext SHALL be built from the governance envelope

`AgentCollaborationPolicyContext`（policy_revision + max_subrun_depth + max_spawn_per_turn）MUST 从治理包络中获取参数。`collaboration_policy_context`（`governance_surface/policy.rs`）从 `ResolvedRuntimeConfig` 构建，由治理装配器统一调用。

#### Scenario: policy context uses governance-resolved parameters

- **WHEN** 系统构建 `AgentCollaborationPolicyContext` 用于协作事实记录
- **THEN** `max_subrun_depth` 和 `max_spawn_per_turn` SHALL 来自治理包络
- **AND** SHALL NOT 从 `ResolvedAgentConfig` 独立读取

### Requirement: spawn budget enforcement SHALL consume governance-resolved limits

`enforce_spawn_budget_for_turn`（`agent/mod.rs`）检查当前 turn 的 spawn 预算。它 MUST 使用治理包络中已解析的限制参数。

#### Scenario: spawn budget check uses envelope parameters

- **WHEN** 系统 spawn 一个新 child 前检查 turn 内 spawn 预算
- **THEN** 预算上限 SHALL 来自治理包络
- **AND** SHALL NOT 从 runtime config 独立读取 `max_spawn_per_turn`

### Requirement: delegation metadata persistence SHALL stay consistent with governance envelope

`persist_delegation_for_handle`（`agent/mod.rs`）将 delegation metadata 持久化到 kernel 控制面。持久化的数据 MUST 与治理包络中的 delegation 信息保持一致。

#### Scenario: persisted delegation matches envelope

- **WHEN** 系统持久化 child 的 delegation metadata
- **THEN** 持久化的数据 SHALL 与治理包络中生成的 delegation 信息一致
- **AND** SHALL NOT 出现持久化数据与治理包络不同步的情况
