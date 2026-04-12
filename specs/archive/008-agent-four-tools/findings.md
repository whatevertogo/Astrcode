# Findings: 当前协作系统现状审计

> **状态**: 实现已完成，以下审计发现已全部解决。保留此文件作为历史决策记录。

## F-001: ~~root agent 尚未注册进 `agent_control` 树~~ ✓ 已解决

**解决方式**: `crates/runtime/src/service/execution/root.rs` 已调用 `register_root_agent(...)`，root agent 正式进入控制树。

## F-002: ~~当前公开协作工具面仍是六工具旧模型~~ ✓ 已解决

**解决方式**: `crates/runtime-agent-tool/src/lib.rs` 和 `crates/runtime/src/builtin_capabilities.rs` 已切换为 `spawn/send/observe/close` 四工具注册。

## F-003: ~~live inbox 已存在，但 durable mailbox 还不存在~~ ✓ 已解决

**解决方式**: 已实现 `AgentMailboxQueued/BatchStarted/BatchAcked/Discarded` 四种 durable mailbox 事件，通过 session event log 追加。

## F-004: 存储层只有单事件 append，没有事务批写和现成 `TurnStarted`

**现状**: 继续保持单事件 append，显式接受 `at-least-once` 语义。该发现的设计决策已落地，无需修改存储层。

## F-005: ~~动态 prompt 注入不会 durable 落进消息历史~~ ✓ 已解决

**解决方式**: mailbox batch 通过 durable 事件持久化，prompt 注入仅作为运行时动态层，不再承担真相源角色。

## F-006: ~~`IndependentSession` 已是默认方向，`SharedSession` 更像历史负担~~ ✓ 已解决

**解决方式**: `ResolvedSubagentContextOverrides::default()` 已将 `storage_mode` 改为 `IndependentSession`，新 child 一律走独立会话。

## F-007: ~~`AgentStateProjector` 不适合作为 mailbox 真相源~~ ✓ 已解决

**解决方式**: mailbox 派生信息通过独立的 replay 逻辑计算，`observe` 快照从 live handle + 对话投影 + mailbox 投影三源聚合。

## F-008: ~~前端和 server 调用层仍显式依赖旧命名~~ ✓ 已解决

**解决方式**: `crates/server/src/http/routes/agents.rs` 和 `frontend/src/lib/api/sessions.ts` 已迁移到新 API 路由和命名。
