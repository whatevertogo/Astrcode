## Purpose

`agent-runtime` 作为最小 live runtime 核心，只负责单次 agent/turn 执行、流式 provider 调用、tool dispatch、hook dispatch、取消、中断与 loop control。它不拥有 session catalog、event log 枚举、branch/fork 持久化、read model、resource discovery、workflow 组装或 settings 解析。

## Requirements

### Requirement: `agent-runtime` SHALL 只拥有最小 live runtime 边界

系统 MUST 新建 `agent-runtime` crate，作为唯一的最小 live runtime 核心。它 SHALL 只负责单次 agent/turn 执行、流式 provider 调用、tool dispatch、hook dispatch、取消、中断与 loop control，SHALL NOT 再拥有 session catalog、event log 枚举、branch/fork 持久化、read model、resource discovery、workflow 组装或 settings 解析。

#### Scenario: runtime core 不再暴露宿主级 session 服务
- **WHEN** 审查 `agent-runtime` 的公共 API
- **THEN** 其中 SHALL 不存在列举全部 session、枚举 session meta、读取 durable replay、发现 prompts/themes/skills 或管理 plugin search path 的入口
- **AND** 这些能力 SHALL 归属 `host-session` 或 `plugin-host`

### Requirement: `agent-runtime` SHALL 消费宿主预解析好的 active snapshot

`agent-runtime` MUST 只消费 host 预先组装好的生效输入，包括模型、工具集合、hook snapshot、provider 选择、prompt 输入与执行约束。runtime SHALL NOT 自己判断贡献来自 builtin 还是 external plugin，也 SHALL NOT 在 turn 中重新做 discovery。

#### Scenario: runtime 不区分 builtin 与 external 贡献来源
- **WHEN** 某次 turn 使用一组工具、hooks、providers 与 prompt declarations 执行
- **THEN** `agent-runtime` 只看到当前生效 snapshot
- **AND** SHALL NOT 因贡献来自 builtin 还是 external 而走不同装配分支

### Requirement: `agent-runtime` SHALL 暴露稳定的 hook/provider/tool 调度点

`agent-runtime` MUST 提供稳定调度点，至少覆盖上下文变换、provider 请求前处理、tool call 前后处理、turn start/end 与取消传播。这些调度点 SHALL 使用纯数据上下文和可组合 effect，而不是暴露 process-local 内部状态。

#### Scenario: runtime 在一次 turn 内顺序执行核心调度点
- **WHEN** 用户提交一轮 prompt
- **THEN** `agent-runtime` SHALL 按 `context -> before_agent_start -> before_provider_request -> tool_call/tool_result -> turn_end` 的顺序驱动调度
- **AND** 每个调度点的输入输出 SHALL 为纯数据 snapshot 或 effect

### Requirement: `agent-runtime` SHALL 只提供单一 turn 执行入口

`agent-runtime` MUST 以单一 turn 执行入口对外暴露其核心能力，例如 `execute_turn(input, cancel)` 这一类语义稳定的方法。系统 SHALL NOT 再把 session catalog、conversation query、event replay、branch/fork 或 config 解析混入 runtime 公共 API。

#### Scenario: 运行时公共面收敛为执行入口
- **WHEN** 审查 `agent-runtime` crate 的正式 surface
- **THEN** 其核心公共能力 SHOULD 收敛到 turn 执行、取消、流式事件与 hook/tool 调度
- **AND** SHALL NOT 再把 session service façade 暴露给上层

### Requirement: `agent-runtime` SHALL 依赖抽象的 provider stream surface

`agent-runtime` MUST 依赖抽象的 provider stream 合同，而不是依赖 `KernelGateway`、`ConfigBackedLlmProvider` 或任何特定 provider kind。runtime SHALL 只知道"如何流式调用模型"，不知道"当前是 OpenAI 还是其他 provider"。

#### Scenario: runtime 不感知 OpenAI-only 现状
- **WHEN** `host-session` 为某次 turn 选择了 provider 并组装执行面
- **THEN** `agent-runtime` 只消费抽象的 stream surface
- **AND** SHALL NOT 在 runtime 内部硬编码 `openai` 或其他 provider kind 分支

### Requirement: `agent-runtime` SHALL 不保留向后兼容 façade

本次重构完成后，系统 SHALL 以 `agent-runtime` 作为新的 live runtime owner，而不是保留旧 `session-runtime` 的兼容 façade 长期并存。

#### Scenario: 旧 monolith runtime 不继续对外提供兼容入口
- **WHEN** `agent-runtime` 与 `host-session` 完成接管
- **THEN** 旧的 monolith `session-runtime` SHALL 不再作为正式对外能力边界继续存在
- **AND** 新实现 SHALL 直接以新 crate 边界为准

### Requirement: `agent-runtime` SHALL 不拥有 collaboration durable truth

`agent-runtime` MAY 提供最小的 child-session 执行合同，但它 MUST 只执行某个 session/turn，而 SHALL NOT 维护 `SubRunHandle`、父子 lineage、input queue、结果投递或取消后的 durable 协作状态。

#### Scenario: 子 agent 执行由 host 触发、runtime 只负责执行
- **WHEN** `host-session` 决定为某个父 turn 启动 child session
- **THEN** `host-session` SHALL 先完成 child session 与协作状态的 durable 建模，再调用 `agent-runtime` 执行该 child turn
- **AND** `agent-runtime` SHALL NOT 自己创建第二套 collaboration truth
