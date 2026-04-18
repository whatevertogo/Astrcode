## ADDED Requirements

### Requirement: shared collaboration protocol SHALL remain centralized while tool descriptions stay low-noise

四工具的 description MUST 优先表达单工具动作语义；共享的 child delegation 协议与 mode-level guidance MUST 保持在专门的共享 guidance surface 中，而不是在每个工具 description 中重复内联。

#### Scenario: shared collaboration guidance is rendered

- **WHEN** 当前 session 同时可使用 `spawn / send / observe / close`
- **THEN** 系统 MUST 提供统一的共享 collaboration guidance
- **AND** 该 guidance MUST 承担四工具的通用决策协议，而不是要求每个工具 description 各自重复解释

#### Scenario: spawn tool description is rendered

- **WHEN** 系统向模型描述 `spawn`
- **THEN** tool description MUST 保持动作导向与低噪音
- **AND** MUST NOT 在 description 内重复内联完整 delegation catalog 或 child execution contract

#### Scenario: send / observe / close descriptions are rendered

- **WHEN** 系统向模型描述 `send`、`observe` 或 `close`
- **THEN** 每个 description MUST 聚焦于该工具的一步动作与边界
- **AND** MUST NOT 重新解释整个 child delegation 心智模型

### Requirement: spawn guidance SHALL distinguish fresh, resumed, and restricted delegation modes

协作 guidance MUST 正式区分 fresh child、resumed child 与 restricted child 三种 delegation mode，并为每种 mode 提供不同的 briefing 规则。

#### Scenario: fresh child is planned

- **WHEN** 模型准备创建一个 fresh child 来承担新的责任边界
- **THEN** guidance MUST 明确要求提供完整任务背景、边界与交付物
- **AND** MUST NOT 将简短催促语句表达为充分的 fresh child briefing

#### Scenario: resumed child is planned

- **WHEN** 模型准备复用一个已有 responsibility continuity 的 child
- **THEN** guidance MUST 明确要求只发送下一条具体指令或澄清
- **AND** MUST NOT 把 resumed child 当成 fresh child 重新完整布置任务

#### Scenario: restricted child is planned

- **WHEN** 模型准备启动一个 capability 收缩后的 child
- **THEN** guidance MUST 明确要求任务分配服从该 child 的 capability surface
- **AND** MUST 明确建议在 capability mismatch 时换用更合适的 child 或当前 agent 自行完成工作
