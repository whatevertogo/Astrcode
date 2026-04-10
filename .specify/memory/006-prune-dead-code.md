---
name: Dead Code & Contract Cleanup
description: 跨层死代码清理、legacy 兼容层删除、subrun canonical contract 收口（已合并）
type: project
---

## 状态 (2026-04-10)
**已合并** — PR #14, `006-prune-dead-code`。一次跨 frontend → protocol → server → runtime → core 的正式支持面清扫。

**Why:** 仓库中长期累积了无人消费的前端读模型/API、后端骨架路由、legacy subrun 兼容路径和重复领域模型，导致维护者无法区分"真实能力"和"历史残留"。

## 核心决策

1. **不保留兼容层** — 删除 `SubRunDescriptor`、optional `parent_turn_id`、descriptorless downgrade、`SubRunOutcome` 双轨、`ChildAgentRef.openable`、外层重复 `open_session_id`
2. **强类型协议** — protocol child/subrun 状态从 `String` 升级为 DTO 枚举，前端不再硬编码字符串集合
3. **唯一事实源** — child open target 只在 `child_ref.open_session_id` 承载；compaction reason → durable trigger 映射集中到单一 owner
4. **共享 payload** — `PromptMetrics` 提取为 storage/agent/protocol 三层复用的共享结构
5. **明确失败** — 对 legacy 输入（descriptorless、legacyDurable）不提供半兼容视图，直接返回结构化错误
6. **cancel cutover** — 前端取消按钮从旧 cancel route 切到 `closeAgent`

## 删除的主要 surface

- 前端：`loadParentChildSummaryList`、`loadChildSessionView`、`ParentSummaryProjection`、`ChildSummaryCard`
- 后端路由：`/children/summary`、`/children/{id}/view`、`/api/v1/agents*`、`/api/v1/tools*`、`/api/runtime/plugins*`、`/api/config/reload`

## How to Apply

- 涉及 subrun 状态/child navigation/compaction trigger 的改动，应使用 canonical contract（`AgentStatus`、`ExecutionAccepted`、`SubRunHandle`）
- protocol DTO 中 child/subrun 状态必须是强类型枚举，不允许退化回字符串
- 不再为旧语义保留 downgrade 路径或兼容映射
