# ADR-0005: Split Policy Decision Plane from Event Observation Plane

- Status: Accepted
- Date: 2026-03-30

## Context

AstrCode 后续必须同时支持两类能力：

- 能改变执行结果的同步决策，例如权限、审批、上下文压力处理
- 只观察运行时事实的异步订阅，例如 UI、SSE、telemetry、audit

如果继续把两者混在一个泛化 hook / wire 机制里，会出现以下问题：

- `Allow / Deny / Ask` 与纯观测事件耦合
- transport 或 UI 容易被误当成核心决策者
- `Ask` 需要挂起与恢复，天然不同于广播观察
- durable event log 与瞬时 runtime event 难以分层

## Decision

冻结 AstrCode 的控制面与观测面为两条不同契约：

### 1. Policy 是唯一同步决策面

Policy contract 拥有改变执行路径的权力，包括：

- 允许
- 拒绝
- 请求审批
- 改写输入
- 决定 context pressure 下的处理策略

### 2. Event 是唯一异步观测面

Event contract 只表达运行时事实，不拥有改变执行结果的权力。

典型用途：

- UI 更新
- SSE 推送
- telemetry
- audit
- debugging

### 3. Approval 通过专门 broker 处理

当 policy 决策返回“需要审批”时：

- runtime 通过专门的 approval broker 挂起并等待结果
- event 层只镜像 `ApprovalRequested` / `ApprovalResolved`
- broker 接口应显式接收 turn cancellation，避免审批挂起与中断语义脱节

审批不通过 EventBus 直接完成 request / response。

### 4. Durable session events 与 runtime observation events 可以不同

AstrCode 可以同时保留：

- 面向 replay / cursor / session 恢复的 durable event
- 面向实时观察者的 runtime agent event

二者可以投影，但不强制相同。

## Consequences

正面影响：

- 权限、审批、context pressure 有了正式控制面
- UI 和 transport 不再被误当成执行仲裁者
- runtime event 可以为多客户端和多协议接入自然复用
- session log 与 runtime observation 的分层更清晰

代价：

- runtime 需要新增 approval broker 一类的显式服务
- 需要定义 policy input / decision 与 event taxonomy
- 一些现有事件流和持久化事件之间需要重新梳理投影关系
