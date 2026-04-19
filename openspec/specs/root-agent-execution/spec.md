## Purpose

为根代理执行入口补齐稳定的控制语义边界，便于应用层与运行时在治理与故障处理层保持一致行为。

## Requirements

### Requirement: `application` 提供根代理执行入口

`application` SHALL 提供正式的根代理执行入口 `execute_root_agent`，将用户请求转化为一次完整的 session 执行，并接受显式执行控制参数（如 `maxSteps`、`tokenBudget`）作为可选输入。

`RootExecutionRequest` 字段：
- `agent_id`: String — 目标 agent 的 profile ID
- `working_dir`: String — 工作目录
- `task`: String — 主任务内容
- `context`: Option<String> — 可选上下文（会与 task 合并为 `{context}\n\n{task}`）
- `control`: Option<ExecutionControl> — 执行控制参数（re-export 自 `astrcode_core::ExecutionControl`）
- `context_overrides`: Option<SubagentContextOverrides> — 根执行当前不支持，非空则报错

`execute_root_agent` 参数：
- `kernel: &dyn AppKernelPort` — kernel 端口
- `session_runtime: &dyn AppSessionPort` — session 端口
- `profiles: &Arc<ProfileResolutionService>` — profile 解析服务
- `governance: &GovernanceSurfaceAssembler` — 治理面组装器
- `request: RootExecutionRequest` — 执行请求
- `runtime_config: ResolvedRuntimeConfig` — 运行时配置

完整流程：参数校验 → profile 解析与 mode 校验 → session 创建 → root agent 注册到控制树 → 治理面组装 → resolved limits 持久化 → task+context 合并 → 异步提交 prompt → 返回 `ExecutionAccepted`

#### Scenario: 执行根代理

- **WHEN** 调用 `execute_root_agent` 并提供合法的 `RootExecutionRequest`
- **THEN** 系统校验参数（agent_id、working_dir、task 非空，control 无 manualCompact），通过 `ProfileResolutionService::find_profile` 加载 profile，校验 mode 为 `Primary` 或 `All`，创建新 session，通过 `kernel.register_root_agent` 注册根 agent，通过 `GovernanceSurfaceAssembler::root_surface` 组装治理面，通过 `kernel.set_resolved_limits` 持久化限制，合并 task + context，通过 `session_runtime.submit_prompt_for_agent` 异步提交，返回 `ExecutionAccepted`（agent_id 设回请求值）

#### Scenario: 非法输入在 application 被拒绝

- **WHEN** `agent_id`、`working_dir` 或 `task` 为空/纯空白
- **THEN** `application` 直接返回 `ApplicationError::InvalidArgument`，错误信息明确提及字段名（agentId / workingDir / task）
- **WHEN** control 中包含 `manualCompact`
- **THEN** 返回 `ApplicationError::InvalidArgument("manualCompact is not valid for root execution")`
- **WHEN** context_overrides 非空非默认
- **THEN** 返回 `ApplicationError::InvalidArgument("contextOverrides is not supported yet for root execution")`
- **AND** 不把错误请求继续下推到 `session-runtime` 或 `kernel`

#### Scenario: 显式执行控制参与根执行

- **WHEN** 调用方在根代理执行请求中提供 `ExecutionControl`（含 maxSteps / tokenBudget）
- **THEN** 系统 SHALL 通过 `control.validate()` 校验，通过治理面解析为 `resolved_limits`，持久化到 kernel 控制树
- **AND** SHALL NOT 仅停留在前端或协议 TODO 字段中

#### Scenario: resolved limits 持久化失败

- **WHEN** `kernel.set_resolved_limits` 返回 None（控制树 handle 已消失）
- **THEN** 返回 `ApplicationError::Internal`，含 agent_id 和失败原因描述

### Requirement: 根代理执行必须通过已解析 profile 驱动

根代理执行 SHALL 基于 working-dir 解析出的 agent profile 进行，而不是在执行过程中临时猜测 profile。

#### Scenario: profile 存在时执行

- **WHEN** 指定 agent 的 profile 可通过 `ProfileResolutionService::find_profile(working_dir, agent_id)` 解析
- **THEN** 系统基于该 profile 发起执行（profile id 传入 `register_root_agent`）

#### Scenario: profile 不存在时失败

- **WHEN** 指定 agent 的 profile 不存在
- **THEN** 返回 `ApplicationError::NotFound`（含 profile id 和 working dir）
- **AND** MUST NOT 创建 session 或注册 agent

#### Scenario: profile mode 不允许根执行

- **WHEN** profile 的 mode 为 `SubAgent`（非 `Primary` 或 `All`）
- **THEN** 返回 `ApplicationError::InvalidArgument`（含 profile id 和 "root execution"）
- **AND** MUST NOT 创建 session 或注册 agent

### Requirement: 治理面参与根执行

根代理执行 SHALL 在提交 prompt 前通过治理面组装器构建完整的治理上下文。

#### Scenario: 治理面组装

- **WHEN** 根执行进入治理面组装阶段
- **THEN** 系统通过 `GovernanceSurfaceAssembler::root_surface` 构建 `RootGovernanceInput`（含 session_id、turn_id、working_dir、profile、mode_id、runtime config、control）
- **AND** 返回的 surface 包含 `resolved_limits` 和 `runtime` 快照
- **AND** resolved_limits 被持久化到 kernel 控制树并写入 handle
