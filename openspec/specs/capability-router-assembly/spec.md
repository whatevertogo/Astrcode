## Purpose

定义 capability router 的统一装配路径，确保 root execution、subagent launch 与 resumed child 三条路径通过同一治理装配器构建能力面。

## Requirements

### Requirement: capability router assembly SHALL follow a unified path across all turn entrypoints

root execution、subagent launch 与 resumed child 三条路径构建 capability router 的逻辑 MUST 统一经过治理装配器，而不是各自独立从不同来源计算能力面。

#### Scenario: root execution resolves capability surface through the governance assembler

- **WHEN** 系统发起一次 root agent execution
- **THEN** root 路径 SHALL 通过统一治理装配器解析当前 turn 的 capability router
- **AND** SHALL NOT 直接在 `execution/root.rs` 中从 `kernel.gateway().capabilities().tool_names()` 独立计算工具面

#### Scenario: subagent launch resolves child-scoped router through the same assembler

- **WHEN** 系统启动一个 fresh child session
- **THEN** subagent 路径 SHALL 通过统一治理装配器生成 child-scoped capability router
- **AND** SHALL NOT 独立在 `execution/subagent.rs:141-172` 中做 `parent_allowed_tools ∩ SpawnCapabilityGrant.allowed_tools` 交集计算

#### Scenario: resumed child resolves scoped router through the shared path

- **WHEN** 父级通过 `send` 恢复一个 idle child
- **THEN** resume 路径 SHALL 通过统一治理装配器生成与 fresh child 一致的 scoped router
- **AND** SHALL NOT 在 `agent/routing.rs:571-722` 中独立构建 capability 子集

### Requirement: capability subset computation SHALL be parameterized by governance envelope, not hardcoded per call site

能力子集的计算参数（parent_allowed_tools、SpawnCapabilityGrant、可见能力面）MUST 从治理包络中统一解析，而不是作为独立参数散落在各调用点。

#### Scenario: child capability grant comes from the governance envelope

- **WHEN** 治理装配器为一个 child turn 生成 capability router
- **THEN** child 的 `SpawnCapabilityGrant` 与 parent 的 allowed_tools SHALL 从治理包络中统一读取
- **AND** SHALL NOT 分别由 `subagent.rs` 和 `routing.rs` 各自从不同参数源构造

#### Scenario: kernel gateway capabilities feed into the governance assembler

- **WHEN** 治理装配器需要当前全局能力面作为输入
- **THEN** 它 SHALL 从 `kernel.gateway().capabilities()` 获取权威来源
- **AND** root/child/resume 路径 SHALL NOT 各自直接调用 kernel gateway 获取能力列表
