## ADDED Requirements

### Requirement: PolicyEngine SHALL consume the resolved governance envelope as its input context

`PolicyEngine` 的三个检查点（`check_model_request`、`check_capability_call`、`decide_context_strategy`）MUST 基于已解析的治理包络做出裁决，而不是在执行路径中保持脱钩状态。

#### Scenario: capability call check uses governance-resolved limits

- **WHEN** turn 执行链路中发生一次能力调用
- **THEN** `check_capability_call` SHALL 能读取当前 turn 治理包络中的 capability surface 和 execution limits
- **AND** SHALL NOT 仅依赖 `PolicyContext` 中与治理包络重复或矛盾的元数据

#### Scenario: model request check is informed by governance envelope

- **WHEN** turn 准备向 LLM 发送请求
- **THEN** `check_model_request` SHALL 能参考治理包络中的 action policy 和 prompt declarations
- **AND** SHALL NOT 在缺少治理上下文的情况下做放行裁决

#### Scenario: context strategy decision aligns with governance envelope

- **WHEN** context pressure 触发上下文策略裁决（compact/summarize/truncate/ignore）
- **THEN** `decide_context_strategy` SHALL 遵守治理包络中可能存在的上下文治理偏好
- **AND** SHALL NOT 始终使用硬编码的默认策略而不考虑治理输入

### Requirement: PolicyContext SHALL be populated from the governance envelope, not independently assembled

`PolicyContext` 当前独立组装 session_id/turn_id/step_index/working_dir/profile 等字段。这些字段 MUST 从治理包络中获取，确保策略引擎的输入与 turn 执行链路使用同一事实源。

#### Scenario: PolicyContext fields align with governance envelope

- **WHEN** 策略引擎需要 `PolicyContext` 做裁决
- **THEN** `PolicyContext` SHALL 从治理包络中派生，而不是在调用点重新组装
- **AND** SHALL NOT 出现 PolicyContext 的 profile 与治理包络的 profile 来源不一致的情况

### Requirement: approval flow types SHALL be connected to the governance assembly path

`ApprovalRequest`、`ApprovalResolution`、`ApprovalPending` 等审批流类型当前仅在 `core/policy/engine.rs` 中定义，没有真实消费者。治理装配路径 SHOULD 为审批流提供明确的接入点，使策略引擎的三态裁决（Allow/Deny/Ask）能在 turn 执行链路中生效。

#### Scenario: Ask verdict triggers approval through the governance path

- **WHEN** 策略引擎对一次能力调用返回 `PolicyVerdict::Ask`
- **THEN** 系统 SHALL 能通过治理装配路径构建 `ApprovalRequest` 并发起审批流
- **AND** SHALL NOT 因缺少接入点而始终回退到 `AllowAllPolicyEngine`

### Requirement: the governance cleanup SHALL preserve AllowAllPolicyEngine as the default while establishing the integration plumbing

本轮 cleanup 不要求实现完整的审批拦截逻辑，但 MUST 确保策略引擎与治理包络之间的接线存在，使得后续 mode system 能通过替换 PolicyEngine 实现来改变治理行为。

#### Scenario: AllowAllPolicyEngine remains the default after cleanup

- **WHEN** 系统在未配置自定义策略引擎的情况下运行
- **THEN** 默认行为 SHALL 继续使用 `AllowAllPolicyEngine` 放行所有请求
- **AND** 治理包络到策略引擎的接线 SHALL 已存在但默认不改变裁决结果

#### Scenario: the integration plumbing allows future PolicyEngine swap without touching turn loop

- **WHEN** 后续 governance mode system 需要实现模式感知的策略裁决
- **THEN** 系统 SHALL 只需替换 PolicyEngine 实现或调整治理包络参数
- **AND** SHALL NOT 需要修改 `run_turn`、tool cycle 或 streaming path
