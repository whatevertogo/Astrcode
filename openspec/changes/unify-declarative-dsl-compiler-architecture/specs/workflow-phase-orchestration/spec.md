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

### Requirement: workflow artifacts SHALL own phase-to-mode binding

workflow phase 与 mode 的关系 MUST 由 workflow artifact 显式持有：phase 通过 `mode_id` 绑定到 mode contract，由治理 compiler / binder 生成治理面；`GovernanceModeSpec` 自身 SHALL NOT 反向声明它属于哪个 workflow phase。

#### Scenario: planning phase resolves its governance through phase mode binding

- **WHEN** `planning` phase 进入执行
- **THEN** 系统 SHALL 通过 `phase.mode_id` 获取对应 mode contract
- **AND** SHALL 由治理编译链路生成该 phase 的 capability surface、prompt 与 artifact / exit 语义
- **AND** SHALL NOT 在 workflow orchestrator 内直接硬编码 plan artifact 或 exit 规则

#### Scenario: the same mode can be reused by multiple phases

- **WHEN** 两个 workflow phase 绑定到同一个 `mode_id`
- **THEN** 系统 SHALL 允许它们复用同一份 mode contract
- **AND** SHALL NOT 因 workflow owner 设计要求为每个 phase 复制一份 mode 定义

#### Scenario: workflow reconcile uses phase-to-mode binding after recovery

- **WHEN** workflow state 已恢复但 mode 状态需要 reconcile
- **THEN** 系统 SHALL 基于 `current_phase_id -> phase.mode_id` 进行 reconcile
- **AND** SHALL NOT 反向从当前 mode 猜测 workflow phase

### Requirement: workflow transition side effects SHALL be owned by application orchestration

workflow phase 迁移附带的业务副作用 MUST 收敛到 application workflow orchestration owner，而不是散落在 tool handler、session-specific helper 和 submit if/else 中。

#### Scenario: plan approval transition owns archive and bridge creation centrally

- **WHEN** planning -> executing 迁移因用户批准而触发
- **THEN** 系统 SHALL 由 application workflow helper 统一执行 plan approval、archive、bridge 生成与 workflow state 持久化
- **AND** SHALL NOT 把这些副作用拆散到 `exitPlanMode`、`session_plan.rs` 与 `session_use_cases.rs` 多处各自维护

#### Scenario: entering plan mode does not bootstrap workflow inside the tool handler

- **WHEN** `enterPlanMode` 触发一次合法的 mode 切换
- **THEN** 工具 handler SHALL 只负责 mode transition 本身
- **AND** workflow bootstrap SHALL 由 application orchestration 在统一边界完成
