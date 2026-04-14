## ADDED Requirements

### Requirement: child execution surfaces SHALL preserve responsibility continuity across launch and reuse

子代理执行链路 MUST 明确保留 child responsibility continuity，帮助上级代理区分“同一责任分支的继续推进”和“创建新的责任分支”。

#### Scenario: fresh child starts a new responsibility branch

- **WHEN** 父级启动一个 fresh child
- **THEN** 系统 MUST 将其视为新的责任分支
- **AND** 后续观察与结果投影 MUST 能区分该分支与其他 child 的责任边界

#### Scenario: resumed child continues the same responsibility branch

- **WHEN** 父级向一个已有 child 发送后续具体指令
- **THEN** 系统 MUST 将其视为同一责任分支上的继续推进
- **AND** MUST NOT 在语义上把这次继续推进伪装成一个新的 fresh delegation

### Requirement: collaboration result projections SHALL expose reuse, close, and respawn decision hints

child 的 observe / terminal result projection MUST 在保留原始事实的同时，补充足以支持 reuse、close 或 respawn 决策的 advisory hints。

#### Scenario: idle child remains reusable for next step

- **WHEN** child 处于 `Idle`，且其当前 responsibility continuity 与 capability surface 仍足以完成预期下一步工作
- **THEN** 结果投影 MUST 明确表达该 child 可以继续通过 `send` 复用
- **AND** MUST 提供简短 reuse hint 或等价的推荐理由

#### Scenario: idle child no longer fits the work

- **WHEN** child 虽然处于 `Idle`，但其 responsibility 边界或 capability surface 已不适合下一步工作
- **THEN** 结果投影 MUST 明确表达存在 responsibility mismatch 或 capability mismatch
- **AND** MUST 允许上级代理据此决定关闭该分支或创建更合适的新 child

#### Scenario: restricted child completes work

- **WHEN** 一个 restricted child 完成最近一轮工作并回到 `Idle`
- **THEN** 结果投影 MUST 仍然保留该 child 的 capability-aware reuse 语义
- **AND** MUST NOT 把“已完成最近一轮工作”错误地表达为“后续任何工作都适合继续复用”
