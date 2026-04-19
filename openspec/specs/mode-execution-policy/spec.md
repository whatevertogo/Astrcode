## Purpose

定义 governance mode 如何影响 turn 级执行限制，包括 max_steps、ForkMode 策略、SubmitBusyPolicy 以及与用户 ExecutionControl 的组合规则。

## Requirements

### Requirement: mode SHALL resolve mode-specific execution limits into the turn envelope

每个 governance mode MUST 在编译 envelope 时解析 mode-specific 的执行限制参数，包括 max_steps、ForkMode 策略、以及 turn 级并发策略（SubmitBusyPolicy），作为 `ResolvedTurnEnvelope` 的一部分。

#### Scenario: execute mode uses default execution limits

- **WHEN** 当前 session 的 mode 为 builtin `code`
- **THEN** envelope 的 execution limits SHALL 与当前默认行为等价
- **AND** max_steps SHALL 来自 runtime config 或用户 `ExecutionControl`

#### Scenario: plan mode uses reduced max_steps

- **WHEN** 当前 session 的 mode 为 builtin `plan`
- **THEN** envelope 的 max_steps SHALL 可由 mode spec 指定上限
- **AND** 用户通过 `ExecutionControl.max_steps` 指定的值 SHALL NOT 超过 mode spec 的上限

#### Scenario: review mode uses minimal execution limits

- **WHEN** 当前 session 的 mode 为 builtin `review`
- **THEN** envelope 的 max_steps SHALL 可限制为 1（仅观察，不执行多步）
- **AND** SHALL NOT 允许 tool chain 执行

### Requirement: mode SHALL determine ForkMode policy for child context inheritance

ForkMode（FullHistory/LastNTurns）决定的上下文继承策略 MUST 受当前 mode 约束。mode spec 可以限制 child 可继承的上下文范围。

#### Scenario: execute mode allows default ForkMode

- **WHEN** 当前 mode 对 child context inheritance 无特殊限制
- **THEN** ForkMode SHALL 按 SpawnAgentParams 的调用参数决定（与当前行为等价）

#### Scenario: restricted mode limits child context to LastNTurns

- **WHEN** 当前 mode 的 child policy 规定 child 只能继承最近 N 个 turn 的上下文
- **THEN** ForkMode SHALL 强制使用 `LastNTurns(N)` 而非 `FullHistory`
- **AND** 即使调用方指定 `FullHistory`，SHALL 被降级为 mode 允许的最大范围

### Requirement: mode SHALL influence SubmitBusyPolicy for turn concurrency

不同 mode 可以有不同的 turn 并发治理策略。mode spec SHALL 能指定当已有 turn 执行时，新 submit 应使用 `BranchOnBusy` 还是 `RejectOnBusy`。

#### Scenario: execute mode uses BranchOnBusy

- **WHEN** 当前 mode 对 turn 并发无特殊限制
- **THEN** SubmitBusyPolicy SHALL 默认为 `BranchOnBusy`（与当前行为等价）

#### Scenario: plan mode uses RejectOnBusy

- **WHEN** 当前 mode 要求 turn 串行执行
- **THEN** SubmitBusyPolicy SHALL 为 `RejectOnBusy`
- **AND** 已有 turn 在执行时的新 submit SHALL 被拒绝而非 branching

### Requirement: mode execution limits SHALL compose with user-specified ExecutionControl

mode 的执行限制 MUST 与用户通过 `ExecutionControl` 指定的限制取交集（更严格者生效），而不是简单覆盖。

#### Scenario: user max_steps is lower than mode limit

- **WHEN** mode spec 允许 max_steps = 50，但用户指定 `ExecutionControl.max_steps = 10`
- **THEN** 实际 max_steps SHALL 为 10（用户限制更严格）

#### Scenario: user max_steps exceeds mode limit

- **WHEN** mode spec 限制 max_steps = 20，但用户指定 `ExecutionControl.max_steps = 50`
- **THEN** 实际 max_steps SHALL 为 20（mode 限制更严格）
- **AND** 系统 SHALL NOT 静默截断，可选择在 submit 响应中提示限制已生效

### Requirement: mode SHALL resolve AgentConfig governance parameters for the current turn

`AgentConfig` 中的治理参数（max_subrun_depth、max_spawn_per_turn 等）MUST 可被 mode spec 覆盖或限制，使不同 mode 能表达不同的协作深度策略。

#### Scenario: plan mode reduces max_spawn_per_turn

- **WHEN** 当前 mode 的 spec 指定 `max_spawn_per_turn = 0`
- **THEN** 该 turn SHALL 不允许 spawn 任何 child
- **AND** spawn 工具 SHALL 不在可见能力面中

#### Scenario: mode does not override AgentConfig by default

- **WHEN** mode spec 未指定覆盖参数
- **THEN** 这些参数 SHALL 使用 runtime config 中的值（与当前行为等价）
