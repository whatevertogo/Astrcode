---
name: 父子会话双层投影视图
description: 003 分支 — 父侧摘要投影 + 子侧完整时间线的双层视图模型，SubRunThreadTree 降级为 legacy
type: project
---

# 父子会话双层投影视图

**Why:** 原有前端使用 `SubRunThreadTree` 从父会话混合消息中构造子执行视图，无法支持独立子会话的直开和持久化。003 分支引入双层投影模型，父侧只看摘要，子侧直开完整 session。

**How to apply:** 前端浏览子 agent 时应使用 `loadChildSessionView(child_session_id)` 直开子会话，父视图中通过 `buildParentSummaryProjection` 展示摘要卡片。

## 父视图模型

父会话中每个 child 以 **summary card** 存在，包含：
- 标题
- child status
- 最新摘要
- 最近工具活动概览
- 最终回复摘录或失败原因
- child session 打开入口

父视图默认隐藏：child 原始 thinking 详情、raw JSON、内部 envelope/transport 细节、child 全量 tool event 序列。

## 子视图模型

子视图是标准 session view，session id 换成 child session：
- 展示 thinking、tool activity、assistant final reply
- 不展示原始 JSON、对 parent 无意义的 transport 元数据

## 页面切换

1. 加载 parent session history/events
2. 从 parent notification 构造 child summary 列表
3. 选中 child 后，直接 `loadSession(child_session_id)`
4. breadcrumb 只记录浏览链路，不参与 durable truth

## 三条交互规则

1. **展开不等于重新构树** — 点击 child card 直接打开目标 child session，不从 parent timeline 重建子树
2. **父摘要与子内容分离** — 父会话只保留摘要；子会话保留全部可读历史；两者共享同一 child ref，不共享消息列表
3. **UI 状态不影响 durable 内容** — 折叠/展开/breadcrumb 只能是前端 read model；刷新后根据 durable child ref 恢复

## 实现位置

| 要素 | 位置 |
|------|------|
| `buildParentSummaryProjection` | `frontend/src/lib/subRunView.ts` |
| 子会话直开 API | `frontend/src/lib/api/sessions.ts`（loadChildSessionView、loadParentChildSummaryList） |
| Server child 投影路由 | `crates/server/src/http/routes/sessions/` |
| `SubRunThreadTree` 降级 | `subRunView.ts` 标注为 legacy |
| Raw JSON 默认隐藏 | `frontend/src/components/Chat/ToolJsonView.tsx` |
| 可折叠 SubRunBlock | `frontend/src/components/Chat/SubRunBlock.tsx` |
| Protocol DTO | `crates/protocol/src/http/agent.rs`（ChildAgentRefDto 等） |
| Server mapper | `crates/server/src/http/mapper.rs`（to_child_agent_ref_dto、to_child_notification_kind_dto） |
| `ChildSessionNotification` SSE 事件 | `crates/server/src/http/mapper.rs`（AgentEventPayload::ChildSessionNotification） |

## Frontend 事件处理

新增 `childSessionNotification` 事件类型（SSE），前端通过 `normalizeAgentEvent` 解析并更新 child summary 列表。测试位于 `frontend/src/lib/agentEvent.test.ts`。

## 手工验收重点

- parent session 中能看到多个 child summary card
- 打开 child session 后能看到 thinking/tool/final reply
- 刷新页面后仍能打开同一个 child session
- 父视图默认不出现 raw JSON
- legacy 历史能明确提示能力受限
