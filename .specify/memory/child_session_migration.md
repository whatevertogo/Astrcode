---
name: 子 Agent 会话迁移计划与状态
description: 003 分支 — 五阶段迁移计划（M1~M5），从 durable foundation 到 legacy cleanup，全部完成
type: project
---

# 子 Agent 会话迁移计划与状态

**Why:** 子 Agent 独立会话涉及 core/protocol/runtime/storage/server/frontend 多层改动，必须分阶段执行以保证每步可验证、可独立评审。

**How to apply:** 003 分支所有五个迁移阶段已完成（2026-04-09）。后续开发可直接基于新架构，无需考虑 legacy 迁移路径。

## M1: Durable Child Session Foundation — 已完成

引入 `ChildSessionNode`、`ChildAgentRef`、`AgentInboxEnvelope`、`CollaborationNotification`，现有 `SubRunHandle`/`SubRunResult` 开始对齐到 durable child-agent ref。

涉及 crates: core、runtime-session、storage、runtime-execution、protocol

## M2: Collaboration Tool Surface + Registry Convergence — 已完成

新增 `sendAgent`、`waitAgent`、`closeAgent`、`resumeAgent`、`deliverToParent`。`CapabilityRouter` 成为唯一生产注册中心，`ToolRegistry` 退化为测试辅助。

涉及 crates: runtime-agent-tool、runtime-registry、runtime、runtime-prompt、core

## M3: Runtime Inbox / Reactivation / Idempotency — 已完成

- inbox/mailbox 投递层落地（`push_inbox`/`wait_for_inbox`）
- child 完成通过 durable envelope + notification 驱动
- parent reactivation（`reactivate_parent_agent_if_idle`）
- close propagation 按 ownership tree 叶子优先（`close_agent_subtree`）
- 去重：相同 dedupe_key 只产生一次有效消费

涉及 crates: runtime-execution、runtime-agent-control、runtime-session、runtime

## M4: Server / Frontend Projection Rewrite — 已完成

- server 提供 child-session / notification 投影
- frontend 直接加载 child session
- parent summary card 替代 mixed subrun thread block
- raw JSON 从默认 UI 移除

涉及: server、frontend/src/lib/api、frontend/src/lib/subRunView.ts、frontend/src/components/Chat

## M5: Delete Legacy Mixed-Session Heuristics — 已完成

- `buildParentSummaryProjection` 独立于 legacy tree
- `cancelSubRun` 标注 legacy
- SSE 错误上下文增强

## Done Criteria 达标情况

全部 7 项满足：
1. child session 和 parent/child ownership 已成为 durable 真相
2. 协作工具族完整可用，模型侧统一通过 tool 调用
3. runtime 内部使用 inbox/mailbox 实现送达、唤醒、恢复和去重
4. parent 会话只消费 notification / summary / tool result projection
5. child 会话可作为独立 session 打开并查看完整 transcript
6. CapabilityRouter 成为唯一生产注册中心
7. mixed-session SubRunThreadTree 主路径已退出生产主流程

## 验证命令

```powershell
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace --exclude astrcode
cd frontend && npm run typecheck && npm run test && npm run lint && npm run build
```

定向验证：
```powershell
cargo test -p astrcode-runtime-agent-tool
cargo test -p astrcode-runtime
cargo test -p astrcode-server
cd frontend && npm run test -- subRunView && npm run test -- agentEvent
```
