# ADR-0008: AgentLoop Content Architecture — Separate Prompt / Context / Compaction / Request Assembly

- Status: Accepted
- Date: 2026-04-03

## Context

`runtime-agent-loop` 中 prompt 组装、上下文选材、历史压缩、请求编码和执行主循环是核心耦合点。为了减少改动时的横向影响，需要把这些内容处理职责显式分离。

## Decision

在 `crates/runtime-agent-loop` 内部把内容处理拆成四个独立模块，让 turn runner 保留执行骨架。

- `PromptRuntime` 负责 prompt 组装与 system prompt 构建，不负责消息裁剪、策略审批或 compact 决策。
- `ContextRuntime` 负责构建当前模型可见的 `ContextBundle`，包括 conversation view、工作集和 memory，不负责 prompt 规则或请求编码。
- `CompactionRuntime` 负责上下文压缩的触发策略、重建和 recovery，不直接负责 prompt 构建或消息发送。
- `RequestAssembler` 负责把 `PromptPlan`、`ContextBundle` 和工具定义编码为最终的 `ModelRequest`，并生成 prompt 快照，不修改上下文内容。
- `turn_runner` 只保留状态机式执行流程，协调上述模块与 LLM/tool cycle。

## Consequences

- prompt、context、compact 和请求编码能够分别演进，减少彼此误伤。
- 替换 compact 策略、上下文阶段或 prompt 层次时，不必改动 AgentLoop 主循环。
- 内部仍然需要清晰界面和测试来避免重新长出新的 God Object。
