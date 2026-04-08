---
name: 子 Agent 独立会话架构
description: 003 分支核心 — 子 Agent 作为独立子会话运行，拥有 durable 真相、完整 transcript、独立生命周期，父 turn 结束不取消子 agent
type: project
---

# 子 Agent 独立会话架构

**Why:** 原有架构中子 agent 作为父 turn 附属执行流，父 turn 结束即取消子 agent，无法支持持久协作和恢复。003 分支将子 agent 重构为独立子会话，拥有自己的 durable 真相和完整生命周期。

**How to apply:** 所有涉及子 agent 创建、生命周期管理、交付和查看的逻辑都应基于本架构。子 agent 不再是父会话消息流中的附属品，而是可独立打开的会话实体。

## 核心数据模型

### ChildSessionNode — Durable 真相节点

记录 child session 的所有权、lineage 和生命周期状态，是 durable truth 的一等公民。

关键字段：
- `agent_id` — 子 agent 稳定身份，tool targeting 和 inbox 投递的主键
- `session_id` / `child_session_id` — 父会话 ID / 子会话 ID
- `sub_run_id` — 稳定执行域 ID，与现有 subrun 兼容
- `parent_session_id` / `parent_agent_id` / `parent_turn_id` — 完整所有权链
- `lineage_kind` — `Spawn | Fork | Resume`，区分创建来源
- `status` — `Pending | Running | Completed | Failed | Cancelled`
- `status_source` — `Live | Durable | LegacyDurable`，状态来源标记
- `created_by_tool_call_id` — 触发创建的协作 tool call

**实现位置:** `crates/core/src/agent/mod.rs`

### ChildAgentRef — 稳定引用

Tool 契约层和 HTTP DTO 共享的稳定 child-agent 引用，由 `ChildSessionNode::child_ref()` 生成。

### ChildSessionNotification — 父侧可消费通知

子会话向父会话投影的结构化通知，包含：notification_id、child_ref、kind（Started/ProgressSummary/Delivered/Waiting/Resumed/Closed/Failed）、summary、status、open_session_id、final_reply_excerpt。

**实现位置:** `crates/core/src/agent/mod.rs`（类型定义）、`crates/runtime-execution/src/subrun.rs`（`build_child_session_notification`）

### AgentInboxEnvelope — 定向投递单元

面向目标 agent 的协作输入投递，支持去重（dedupe_key）和投递生命周期（queued/delivered/consumed/superseded/failed）。

### ChildSessionExecutionBoundary — 执行边界

子会话自有的运行策略：run_mode、storage_mode、approval_policy、tool_scope、working_dir、isolation、profile_id。

## 关键设计原则

1. **父 turn 结束不取消子 agent** — 子 agent 独立于父 turn 存活，只有显式关闭、取消或自身终态才能结束
2. **父会话只接收通知** — 父 history 只保留 ChildSessionNotification，不混入子 transcript
3. **Ownership 不从磁盘路径推断** — parent/child 关系由 ChildSessionNode 显式记录
4. **Status source 三级区分** — Live/Durable/LegacyDurable 明确标记状态来源
5. **Legacy 降级** — 没有 ChildSessionNode 的历史数据显式降级，不伪造 parent-child 关系

## 边界与 Owner 分配

| Boundary | Owns | Must not own |
|---------|------|--------------|
| `core` | ChildAgentRef、协作 tool DTO、inbox/notification 领域类型 | runtime 默认值、tool 执行实现 |
| `runtime-agent-tool` | 协作工具适配、schema、结果映射 | child session orchestration、inbox 存储 |
| `runtime-execution` | child session orchestration、投递/唤醒/去重、handoff 组装 | session durable ledger |
| `runtime-agent-control` | live agent handle、cancel token、运行态状态控制 | durable truth |
| `runtime-session` / `storage` | session durable ledger、JSONL、child session 节点与通知写入 | live handle 控制 |
| `server` | child session / parent summary DTO 投影与 HTTP/SSE 路由 | durable 真相判断 |
| `frontend` | 父摘要视图、子会话完整视图、breadcrumb | 反推 parent/child ownership 真相 |

## 关键实现文件

- `crates/core/src/agent/mod.rs` — ChildSessionNode、ChildAgentRef、ChildSessionNotification 类型
- `crates/runtime-execution/src/subrun.rs` — build_child_session_node、build_child_session_notification
- `crates/runtime/src/service/execution/subagent.rs` — 子 agent 创建、child node 写入、通知发射
- `crates/runtime/src/service/execution/status.rs` — project_child_terminal_delivery 终态投影
- `crates/protocol/src/http/agent.rs` — ChildAgentRefDto、ChildSessionNotificationKindDto 等 DTO
- `crates/server/src/http/mapper.rs` — core → protocol DTO 映射
