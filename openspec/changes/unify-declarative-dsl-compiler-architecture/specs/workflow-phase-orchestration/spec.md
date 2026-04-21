## ADDED Requirements

### Requirement: workflow definitions SHALL be validated and compiled before orchestration

正式 workflow 在进入 `WorkflowOrchestrator` 前 MUST 先经过显式校验与轻量编译，形成可被 application 消费的 workflow artifact。该 compiled workflow artifact SHALL 保留 phase、transition、signal 与 bridge 语义，但当前规模下 MUST NOT 强制引入额外索引结构。

#### Scenario: builtin workflow is validated before orchestration

- **WHEN** 系统装载 builtin `plan_execute` workflow
- **THEN** 它 SHALL 先校验 initial phase、phase 引用、transition 来源/目标与 signal 合法性
- **AND** 仅在校验通过后才进入 orchestration 路径

#### Scenario: compiled workflow artifact keeps the existing vector-oriented shape

- **WHEN** 当前 workflow 规模仍然很小
- **THEN** 系统 MAY 继续以 `Vec` 形状承载 phase 与 transition
- **AND** SHALL NOT 为了满足 compile artifact 概念而强制引入与当前规模不匹配的索引化结构

### Requirement: workflow binding SHALL explicitly reference mode contracts rather than re-encoding mode behavior

workflow phase 与 mode 的关系 MUST 通过显式 binding 表达：phase 绑定到 mode contract，由 governance compiler / binder 负责生成治理面；workflow 自身 SHALL NOT 重新编码 capability surface、artifact gate 或 prompt 行为。

#### Scenario: planning phase binds to a mode contract instead of inlining plan semantics

- **WHEN** `planning` phase 进入执行
- **THEN** 系统 SHALL 通过 phase -> mode binding 获取对应 mode contract
- **AND** SHALL 由治理编译链路生成该 phase 的 capability surface、prompt 与 artifact gate
- **AND** SHALL NOT 在 workflow orchestrator 内直接硬编码 plan artifact 或 exit 规则

#### Scenario: workflow reconcile uses phase-to-mode binding after recovery

- **WHEN** workflow state 已恢复但 mode 状态需要 reconcile
- **THEN** 系统 SHALL 基于 `current_phase_id -> mode binding` 进行 reconcile
- **AND** SHALL NOT 反向从当前 mode 猜测 workflow phase
