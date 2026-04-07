# ADR-0008: AgentLoop 内容架构——Prompt/Context/Compaction/Assembler 四层分离

- Status: Accepted
- Date: 2026-04-03

## Context

重构前的 `runtime-agent-loop` 把 prompt 组装、上下文选材、历史压缩、请求编码和执行主循环揉在一起，导致替换 compact 策略、开放上下文选材或调整 prompt 结构时都必须修改主执行流程，边界持续模糊。

## Decision

在 `runtime-agent-loop` 内部将内容处理拆为四个独立职责，并让 turn runner 只保留执行骨架。

- `PromptRuntime` 只负责说明书和指令结构，不负责历史消息选材或压缩决策。
- `ContextRuntime` 只负责决定当前向模型提供哪些材料，不负责 policy、compact 或请求装配。
- `CompactionRuntime` 只负责历史折叠、压缩策略和压缩后视图恢复。
- `RequestAssembler` 作为唯一请求装配边界，只负责把 prompt、context 和 tool definitions 编码为模型请求。
- `turn_runner` 只保留状态机式执行流程，协调上述运行时与 LLM/tool cycle。
- 在抽象稳定前，先维持模块内分层，不急于拆分新 crate。

## Consequences

- prompt、context、compaction 和请求编码可以分别演进，减少彼此误伤。
- 替换 compact 策略、上下文阶段或 prompt 结构时，不必再改动整条主循环。
- 内部会引入更多中间抽象，需要通过清晰边界避免重新长成新的 God Object。
