## Purpose

定义执行限制与控制参数（ResolvedExecutionLimitsSnapshot、ExecutionControl、ForkMode、SubmitBusyPolicy）如何通过治理装配路径统一解析，消除分散计算。

## Requirements

### Requirement: execution limits SHALL be resolved as part of the unified governance envelope

`ResolvedExecutionLimitsSnapshot`、`ExecutionControl`、`ForkMode` 与 `SubmitBusyPolicy` 等执行限制与控制输入 MUST 在治理装配阶段统一解析为治理包络的一部分，而不是在提交路径中各自独立计算。

#### Scenario: root execution limits come from the governance assembler

- **WHEN** 系统发起一次 root agent execution
- **THEN** `ResolvedExecutionLimitsSnapshot`（allowed_tools + max_steps）SHALL 由治理装配器生成
- **AND** SHALL NOT 在 `execution/root.rs:71-85` 中独立从 kernel gateway 和 `ExecutionControl.max_steps` 计算

#### Scenario: child execution limits come from the governance assembler

- **WHEN** 系统启动或恢复一个 child session
- **THEN** child 的 `ResolvedExecutionLimitsSnapshot` SHALL 由治理装配器根据 child policy 与 parent limits 统一计算
- **AND** SHALL NOT 在 `execution/subagent.rs:141-172` 中独立做 allowed_tools 交集运算

#### Scenario: ExecutionControl feeds into the governance assembler, not directly into submission

- **WHEN** 用户通过 `submit_prompt_with_control` 提交一个带 `ExecutionControl` 的请求
- **THEN** `ExecutionControl` 的 max_steps 与 manual_compact SHALL 作为治理装配器的输入参数
- **AND** SHALL NOT 直接在 `session_use_cases.rs:125-134` 中覆写 runtime config

### Requirement: AgentConfig governance parameters SHALL flow through the governance assembly path

`max_subrun_depth`、`max_spawn_per_turn`、`max_concurrent_agents` 等 `AgentConfig` 治理参数 MUST 通过治理装配路径统一传递到消费方，而不是通过 runtime config 在各消费点分散读取。

#### Scenario: spawn budget enforcement uses governance-resolved parameters

- **WHEN** `enforce_spawn_budget_for_turn` 检查当前 turn 的 spawn 预算
- **THEN** 它 SHALL 使用治理包络中已解析的 spawn 限制参数
- **AND** SHALL NOT 直接从 `ResolvedAgentConfig` 中分散读取 `max_spawn_per_turn`

#### Scenario: collaboration policy context is built from the governance envelope

- **WHEN** `AgentCollaborationPolicyContext` 需要构建用于协作事实记录
- **THEN** `max_subrun_depth` 和 `max_spawn_per_turn` SHALL 来自治理包络
- **AND** SHALL NOT 在 `agent/mod.rs:741-749` 中独立从 runtime config 读取

### Requirement: ForkMode and context inheritance SHALL be governed by the unified assembly path

`ForkMode`（FullHistory/LastNTurns）决定的上下文继承策略 MUST 作为治理包络的一部分，而不是在 `subagent.rs:247-297` 中作为独立逻辑处理。

#### Scenario: child context inheritance strategy comes from the governance envelope

- **WHEN** 系统为 child 选择继承的父级上下文范围
- **THEN** ForkMode 的选择与 recent tail 裁剪逻辑 SHALL 由治理装配器驱动
- **AND** SHALL NOT 在 `select_inherited_recent_tail` 中独立实现

### Requirement: SubmitBusyPolicy SHALL be derivable from the governance envelope

`SubmitBusyPolicy`（BranchOnBusy/RejectOnBusy）当前硬编码在 session-runtime，但其语义是 turn 级并发治理策略，MUST 可以被治理包络覆盖。

#### Scenario: default busy policy is derived from governance configuration

- **WHEN** 系统 submit 一个 prompt 且已有 turn 在执行
- **THEN** busy policy SHALL 可从治理包络中读取，而不是固定为 `BranchOnBusy`
- **AND** 不同入口类型（root vs subagent vs resumed）SHALL 可以有不同的默认 busy policy
