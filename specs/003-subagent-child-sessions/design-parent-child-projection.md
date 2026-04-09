# Design: Parent/Child 会话投影与前端视图

## 目标

把当前基于 `SubRunThreadTree` 的 mixed-session 浏览模型替换成以下双层视图：

1. **父视图**：只展示 child summary、关键工具活动、状态、最终回复摘录和打开入口
2. **子视图**：直接加载 child session transcript，展示 thinking、tool activity 和最终回复

## 父视图模型

父会话中的每个 child 以 summary card 存在，而不是一段被嵌入 parent timeline 的原始消息流。

每个 summary card 至少包含：

- 标题
- child status
- 最新摘要
- 最近工具活动概览
- 最终回复摘录或失败原因
- child session 打开入口
- 可选的等待、关闭、继续协作动作

父视图默认隐藏：

- child session 的原始 thinking 文本详情
- raw JSON
- 内部 envelope / transport 细节
- child 全量 tool event 序列

## 子视图模型

子视图是标准 session view，只是 session id 换成 child session。

默认展示：

- thinking
- tool activity
- assistant final reply
- 子会话状态和 lineage 提示

不展示：

- 原始 JSON
- 对 parent 无意义的 transport 元数据

## 页面切换与加载方式

### 新模型

1. 加载 parent session history/events
2. 从 parent notification 构造 child summary 列表
3. 选中某个 child 后，直接 `loadSession(child_session_id)`
4. breadcrumb 只记录当前浏览链路，不参与 durable truth 判断

### 旧模型的迁移目标

当前 `frontend/src/lib/subRunView.ts` 会把父会话中的 mixed messages 构造成 `SubRunThreadTree`。  
新模型要求：

- `SubRunThreadTree` 退化为 legacy fallback 或测试辅助
- parent/child 关系不再通过 mixed message 重建
- child transcript 的来源变成 child session 自己的 history/events

## 服务端投影要求

server 需要提供稳定的投影输入，前端不应自己猜：

- parent history/events 中出现 child summary / notification
- child status 查询返回稳定 child ref
- child session 可以像普通 session 一样被历史接口直接加载
- legacy 历史显式标注 lineage 缺失，而不是伪造 parent-child 关系

## 交互规则

### 规则 1: 展开不等于重新构树

用户点击 child card 时，应直接打开目标 child session，而不是从 parent timeline 重新过滤或重建一棵子树。

### 规则 2: 父摘要与子内容分离

父会话只保留用于继续决策的摘要；子会话保留全部可读历史。  
这两个视图共享同一 child ref，但不共享同一消息列表。

### 规则 3: UI 状态不影响 durable 内容

折叠、展开、breadcrumb、active path 都只能是前端 read model；刷新后应根据 durable child ref 恢复，而不是把 UI 状态写成领域真相。

## 手工验收重点

- parent session 中能看到多个 child summary card
- 打开 child session 后能看到 thinking/tool/final reply
- 刷新页面后仍能打开同一个 child session
- 父视图默认不出现 raw JSON
- legacy 历史能明确提示能力受限，而不是显示错误 parent-child 结构

---

## 实现确认（2026-04-09）

以上投影设计已全部落地：

| 设计要素 | 实现位置 |
|---------|---------|
| 父摘要卡片（`ChildSummaryCard`） | `frontend/src/lib/subRunView.ts`（buildParentSummaryProjection） |
| 子会话直开 API | `frontend/src/lib/api/sessions.ts`（loadChildSessionView、loadParentChildSummaryList） |
| Server child 投影路由 | `crates/server/src/http/routes/sessions/query.rs`（children summary / child view） |
| `SubRunThreadTree` 降级为 legacy | `subRunView.ts` 标注为 legacy，新摘要投影直接从索引构建 |
| Raw JSON 默认隐藏 | `frontend/src/components/Chat/ToolJsonView.tsx`（移除默认 raw JSON 渲染） |
| 可折叠 SubRunBlock | `frontend/src/components/Chat/SubRunBlock.tsx`（thinking / tool / final-reply 三区折叠） |
| Lineage 兼容 | spawn / fork / resume 三种 `lineage_kind` 统一投影，新增 `LineageSnapshot` |
