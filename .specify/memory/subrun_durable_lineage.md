---
name: Subrun Durable Lineage Protocol
description: Durable history is truth for completed subruns, descriptor/tool_call_id mandatory for new writes
type: protocol
---

# Subrun Durable Lineage Protocol

## 核心原则 (不可违反)

**Durable history = 已完成 subrun 的唯一真相**。Live state 仅补充运行中状态。

## 必须写入的字段

`SubRunStarted` / `SubRunFinished` 必须包含：
- `descriptor: SubRunDescriptor` (sub_run_id, parent_turn_id, parent_agent_id, depth)
- `tool_call_id: String`

## Legacy 降级策略 (不可伪造)

- 旧历史缺 `descriptor` → 返回 `source=legacyDurable`，`descriptor=None`
- **Lineage 依赖型 scope 过滤直接失败**，不推断父子关系
- 不提供批量回填脚本

## 查询优先级

1. Live control registry (运行中)
2. Durable lifecycle events (已完成)
3. 缺 descriptor → `legacyDurable` + UI 降级展示

## 存储模式无关性

共享会话与独立会话必须暴露相同的父子执行语义，归属与存储位置解耦
