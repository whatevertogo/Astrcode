# AstrCode Architecture

已接受且仍然生效的 ADR：

- [ADR-0001](../adr/0001-astrcode-protocol-v4-wire-model.md) — Protocol V4 wire model 冻结
- [ADR-0002](../adr/0002-astrcode-coding-profile-context.md) — Coding Profile 上下文边界冻结
- [ADR-0003](../adr/0003-unified-capability-routing.md) — 统一 Capability 路由模型
- [ADR-0004](../adr/0004-layered-core-runtime-transport-architecture.md) — 三层分层架构边界冻结
- [ADR-0005](../adr/0005-split-policy-decision-plane-from-event-observation-plane.md) — Policy 控制面与 Event 观测面分离
- [ADR-0006](../adr/0006-turn-outcome-state-machine.md) — TurnOutcome 状态机化 + 移除 max_steps
- [ADR-0007](../adr/0007-layered-prompt-builder-for-kv-cache-optimization.md) — 分层 Prompt 构建器（设计完成，未投入生产）
- [ADR-0008](../adr/0008-agent-loop-content-architecture.md) — AgentLoop 四层内容架构（Prompt/Context/Compaction/Assembler 分离）

架构文档：

- [architecture.md](./architecture.md) — 三层架构与四类核心契约（crate 依赖图、设计规则）
- [skills-architecture.md](./skills-architecture.md) — Claude 风格两阶段 skill 架构（目录格式、Skill tool、资源模型）
- [agent-loop-roadmap.md](./agent-loop-roadmap.md) — AgentLoop 演进计划（P1-P4 已完成 + 远期 TODO）
- [frontend-architecture.md](./frontend-architecture.md) — React 前端 SPA 架构（状态管理、SSE、认证、组件树）
