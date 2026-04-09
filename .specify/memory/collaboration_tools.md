---
name: 子 Agent 协作工具契约
description: 003 分支 — 六个协作工具（spawn/send/wait/close/resume/deliver）的契约层定义、约束与 runtime 映射
type: project
---

# 子 Agent 协作工具契约

**Why:** 原来只有 `spawnAgent` 和 `cancel`，无法支持持续协作、等待、恢复、交付。003 分支扩展为完整的六工具族，统一模型侧交互。

**How to apply:** 所有主子 agent 协作在模型侧统一表现为 tool 调用。Runtime 内部通过 inbox/mailbox 定向投递实现实际送达与唤醒，不把内部 transport 暴露给模型。

## 工具族概览

| 工具 | 用途 | 可见性 |
|------|------|--------|
| `spawnAgent` | 创建新 child agent / child session | 父 agent |
| `sendAgent` | 向既有 child agent 追加要求或返工请求 | 父 agent |
| `waitAgent` | 等待指定 child agent 到达可消费状态 | 父 agent |
| `closeAgent` | 关闭指定 child agent 或其子树 | 父 agent |
| `resumeAgent` | 恢复已完成但可继续协作的 child agent | 父 agent |
| `deliverToParent` | 把结果送回直接父 agent | 仅子 agent |

## 三个核心约束

1. **输入约束** — 只描述目标 agent、意图和补充上下文，不描述 runtime 内部 transport 细节
2. **输出约束** — 必须是可消费的稳定结果语义，不返回 runtime 原始 envelope 或 raw JSON
3. **投递约束** — 只触发"向某个目标 agent 投递一个 envelope"，不直接修改其他 agent 内部状态

## 各工具关键参数

### spawnAgent
- Input: `prompt`(必填)、`description`(必填)、`type`(可选 profile)、`context`(可选)
- Output: success、agentRef、status=running、summary、openSessionId

### sendAgent
- Input: `agentId`(必填)、`message`(必填)、`context`(可选)
- Output: success、accepted、agentRef、deliveryId

### waitAgent
- Input: `agentId`(必填)、`until`(可选，final/next_delivery)
- Output: status、agentRef、summary、finalReply 或 failure

### closeAgent
- Input: `agentId`(必填)、`cascade`(可选，默认 true)
- Output: accepted、closedRootAgentId、cascade

### resumeAgent
- Input: `agentId`(必填)、`message`(可选)
- Output: accepted、agentRef、status=running

### deliverToParent
- Input: `summary`(必填)、`findings`(可选)、`finalReply`(可选)、`artifacts`(可选)
- Output: accepted、parentAgentId、deliveryId

## Runtime 内部映射

这些工具在 runtime 内部映射到：
- durable `AgentInboxEnvelope`
- 目标 agent 唤醒 / 排队
- 必要时的 `CollaborationNotification`

关键实现：
- `crates/runtime-agent-tool/src/` — 工具适配和结果映射
- `crates/runtime-agent-control/src/lib.rs` — push_inbox / wait_for_inbox
- `crates/runtime/src/service/execution/subagent.rs` — deliver_to_parent

## 幂等与去重

相同 `dedupe_key` 对同一 `target_agent_id` 只产生一次有效消费，恢复/重试不重复执行。

## Registry 收口

- `CapabilityRouter` 成为唯一生产执行注册中心
- `ToolRegistry` 退化为测试/装配辅助
- 所有协作工具通过 CapabilityRouter 注册
