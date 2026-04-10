# Contract: Session History And Child Notifications

## 目的

定义父会话历史、子会话历史、`/history`、`/events` 以及父侧通知投影之间的边界契约。

## 1. Parent Session Contract

父会话 durable 历史中只允许出现对子智能体可追溯、可消费的边界事实，例如：

- `child_started`
- `child_resumed`
- `child_delivered`
- `child_completed`
- `child_failed`
- `child_cancelled`
- `child_lineage_mismatch`

每条边界事实至少要能映射出：

- `childSessionId`
- `executionId`
- `status`
- `summary`
- `openSessionId`

## 2. Parent Session Must Not Contain

父会话 durable 历史不得包含：

- 子会话完整 transcript
- 子会话内部事件，如 `AssistantFinal`、`ToolCall`、`ToolResult`、`PromptMetrics`、`TurnDone`
- `ReactivationPrompt` 或等价机制性 `UserMessage`
- 作为下一轮桥接使用的交付详情原文

## 3. Child Session Contract

子会话 durable 历史保留自己的完整内部过程，包括：

- 消息历史
- 工具活动
- 摘要/压缩相关记录
- 最终回复
- resume 所需的 durable 材料

调用方必须能直接按 `childSessionId` 读取 child 的 `/history` 与 `/events`，而不是要求先从父历史中过滤。

## 4. `/history` And `/events` Projection Contract

- 两个接口必须对同一边界事实表达一致语义。
- `/history` 可以更偏阅读体验，`/events` 可以更偏事件流，但都不能重新引入 child 内部 transcript 到父视角。
- 若 lineage 不一致，两个接口都必须显式暴露失败，而不是静默略过。

## 5. Restart And Traceability Contract

- 若一次性交付输入在父消费前因重启丢失，父历史里仍必须保留“交付发生过”的 durable 边界事实。
- 子会话入口必须稳定可打开，便于追溯交付来源。

## 6. Legacy Shared History Contract

- 旧共享写入历史不再属于受支持输入。
- 调用方若请求读取或恢复旧共享写入历史，系统必须返回稳定错误码 `unsupported_legacy_shared_history`。
- 新写入不得继续落入共享写入模式。
