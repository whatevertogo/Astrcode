# Contract: Summary 与 Child Navigation 收口

本合同定义清理后保留哪些 summary 语义，以及 child navigation 依赖什么来完成。

## 1. 保留的 Summary

### `SubRunHandoff.summary`

- 语义：子 Agent 完成时交给父流程或 UI 的终态摘要
- 状态：保留
- 原因：当前产品路径真实消费

### `ChildSessionNotification.summary`

- 语义：父侧通知事件中的可读摘要
- 状态：保留
- 原因：当前父侧展示与 child session 入口真实消费

## 2. 删除的 Summary Projection

以下能力删除：

- parent-child summary list API
- child session view projection API
- `buildParentSummaryProjection`
- `ParentSummaryProjection`
- `ChildSummaryCard`

原因：它们只是重复投影，没有当前消费者。

## 3. Child Navigation 的正式来源

清理后，child navigation 只依赖以下事实：

- 当前会话历史 / SSE 中已有的 child-related 事件与消息
- `child_ref.open_session_id` 这类 canonical open target
- focused subrun 浏览辅助树
- durable child session fact（例如 `child_session_id`）

不再依赖：

- 专门的 parent-summary route
- 专门的 child-view route
- duplicated `openable` bool
- 通知或 DTO 外层重复 `open_session_id`

## 4. 规则

- `ChildAgentRef` 描述身份、lineage、状态和唯一 canonical open target。
- “能不能打开”由是否存在 canonical open target 决定，而不是额外布尔字段。
- `ChildSessionNotification` 如需暴露打开目标，只能通过嵌套 `child_ref.open_session_id`。
- 任何新的 child navigation 入口都必须来自 canonical fact，而不是新的 projection API。

## 5. 禁止事项

- 不得因为删除 projection，就把仍在使用的 summary fact 一起删除。
- 不得因为 child navigation 仍存在，就把无人消费的 summary API 一并保留。
