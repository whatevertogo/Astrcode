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

- 当前会话历史 / SSE 中已经存在的 child-related 事件与消息
- child session 的直接可打开标识
- 当前 focused subrun 浏览辅助树

不再依赖：

- 专门的 parent-summary route
- 专门的 child-view route
- 无消费者的 projection 层

## 4. 禁止事项

- 不得因为删除 projection，就把仍在使用的 summary facts 一起删除。
- 不得因为 child navigation 仍存在，就把无人消费的 summary API 一并保留。
