# Findings: 当前协作系统现状审计

## F-001: root agent 尚未注册进 `agent_control` 树

**Evidence**:
- `crates/runtime/src/service/execution/root.rs` 会生成 `root_agent_id`
- 但 `resolve_parent_execution` 对 `RootExecution` 仍返回 `parent_agent_id_for_control = None`

**Impact**:
- 根层 child 没有稳定直接父 agent 控制身份
- `send(parentId, ...)`、`observe(childId)` 和统一权限校验在根层缺少真实 owner

## F-002: 当前公开协作工具面仍是六工具旧模型

**Evidence**:
- `crates/runtime-agent-tool/src/lib.rs`
- `crates/runtime/src/builtin_capabilities.rs`
- `crates/runtime-prompt/src/contributors/workflow_examples.rs`

**Impact**:
- prompt 仍在引导模型使用 `waitAgent` / `sendAgent` / `deliverToParent` / `resumeAgent`
- 公开面与新 spec 的四工具目标明显冲突

## F-003: live inbox 已存在，但 durable mailbox 还不存在

**Evidence**:
- `crates/runtime-agent-control/src/lib.rs` 已持有 `inbox`、`parent_delivery_queues`
- 但协作消息主要停留在 live 内存结构，没有 durable mailbox 事件

**Impact**:
- 运行时重启后 pending message 无法通过 durable 事件完整恢复
- 现有 parent delivery 更像一次性通知，而不是通用 mailbox

## F-004: 存储层只有单事件 append，没有事务批写和现成 `TurnStarted`

**Evidence**:
- `runtime-session` 当前提供的是单事件追加
- 仓库中没有现成 `TurnStarted` durable 事件和批事务 API

**Impact**:
- `BatchStarted` / `Acked` 无法依赖事务合并实现 exactly-once
- 需要显式接受 `at-least-once`

## F-005: 动态 prompt 注入不会 durable 落进消息历史

**Evidence**:
- 当前 parent delivery 与声明式 prompt 注入走 `PromptDeclaration`
- 注入内容不会自动形成 durable `UserMessage`

**Impact**:
- 不能依赖 context window 一定保留历史 `delivery_id`
- 服务端必须先做批内去重和 replay 定义，不能把重放语义推给模型“记忆”

## F-006: `IndependentSession` 已是默认方向，`SharedSession` 更像历史负担

**Evidence**:
- `crates/runtime-execution/src/policy.rs` 已把 `IndependentSession` 作为主方向
- 但旧路径仍保留 `SharedSession` 可表达性

**Impact**:
- 新四工具模型完全可以统一到 `IndependentSession`
- 继续为新 child 写 `SharedSession` 只会放大迁移复杂度

## F-007: `AgentStateProjector` 不适合作为 mailbox 真相源

**Evidence**:
- 现有 projector 主要投影 phase、turn_count、assistant 输出等对话状态
- mailbox 需要 pending/active batch、acked/discarded 等不同维度

**Impact**:
- 如果把 mailbox 状态强塞进去，会让边界 owner 混乱
- `observe` 需要一个新的 mailbox 派生来源

## F-008: 前端和 server 调用层仍显式依赖旧命名

**Evidence**:
- `frontend/src/hooks/useAgent.ts`
- `frontend/src/lib/api/sessions.ts`
- `crates/server/src/tests/session_contract_tests.rs`

**Impact**:
- 即使 runtime 内部完成重构，调用层仍会把旧工具名暴露出去
- 迁移必须包含 server/frontend，而不是只改 runtime crate
