# Design: 支持面清点与 Surface Pruning

## 1. 目标

本设计定义本次 feature 如何识别、分类并删除当前仓库中的 orphan surface、legacy surface 和重复 projection surface，
同时明确哪些能力仍属于当前正式支持面。

## 2. 分类规则

### 2.1 明确保留

满足以下条件才允许保留：

- 当前主线产品或运行时有真实消费者
- 能明确指出 owner boundary
- 不与另一条正式入口表达同一语义

### 2.2 立即删除

符合以下任一条件即直接删除：

- 没有真实消费者，只剩测试或文档引用
- 只是 skeleton / placeholder / 预埋接口
- 只是重复 projection，没有主线消费
- 只是为了 legacy downgrade 或半兼容显示保留

### 2.3 迁移后删除

仅当某入口仍是当前活跃流程唯一入口时，允许先迁移后删除：

- 当前 UI 仍直接调用旧控制入口
- 当前回归场景仍依赖旧 route
- 替代入口已经存在且可验证

## 3. Surface Inventory

### 3.1 明确保留

- 当前会话历史与 SSE 主线
- 当前消息提交 / 中断 / compact / 删除
- focused subrun 浏览
- child session 直开
- 当前配置读写、模型枚举与连通性测试
- 当前 handoff summary / child notification summary

### 3.2 立即删除

- `loadParentChildSummaryList`
- `loadChildSessionView`
- `buildParentSummaryProjection`
- `ParentSummaryProjection`
- `ChildSummaryCard`
- `/api/sessions/{id}/children/summary`
- `/api/sessions/{id}/children/{child_session_id}/view`
- `/api/v1/agents*`
- `/api/v1/tools*`
- `/api/runtime/plugins*`
- `/api/config/reload`
- `openable`
- notification / DTO 外层重复 `open_session_id`
- protocol `status: String`

### 3.3 迁移后删除

- 旧 cancel route 与前端包装
- 任何仍被当前 UI 按钮调用的 legacy close/cancel 路径

## 4. 删除策略

### 4.1 删除 orphan surface 时的同步要求

- 同步删除类型、mapper、route、测试、夹具和文档引用
- 不允许只删调用方而保留空壳 surface

### 4.2 删除 duplicated surface 时的同步要求

- 指定唯一 canonical target
- 调用方先切到 canonical target
- 再删除旧字段、旧 DTO、旧 helper 和旧断言

### 4.3 不能做的事情

- 不因“以后可能会用”保留 public surface
- 不通过 adapter 或 alias 继续保留已判死的 surface
- 不把 UI 便利字段重新塞回 core/protocol

## 5. Summary 收口原则

### 5.1 保留

- `SubRunHandoff.summary`
- `ChildSessionNotification.summary`

### 5.2 删除

- parent-child summary projection API
- child session view projection API
- 任何没有当前消费者的平行 summary 读模型

### 5.3 Guardrails

- 不得因为删除 projection，而误删仍被主线消费的摘要事实
- 不得因为 child navigation 仍存在，就保留无人消费的 summary API

## 6. Child Navigation 收口原则

- child navigation 的正式来源是 `child_ref.open_session_id` 与 durable child fact
- `openable` 不再作为正式 surface 存在
- 通知与 DTO 外层不再重复存 `open_session_id`
- 不再通过 legacy summary/view route 补导航信息

## 7. Validation Focus

- 所有保留 surface 都能指出真实消费者与 owner
- 所有已删除 surface 在代码、文档、测试中都不再出现
- child navigation 继续可用，但只依赖 canonical open target
