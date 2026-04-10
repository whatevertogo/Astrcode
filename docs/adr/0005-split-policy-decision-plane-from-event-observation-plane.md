# ADR-0005: Split Policy Decision Plane from Event Observation Plane

- Status: Accepted
- Date: 2026-03-30

## Context

AstrCode 既要做可影响执行结果的同步决策，也要做独立的运行时事实观测。若把两者混用，审批、权限控制和 UI/telemetry 观测会相互耦合，导致执行控制语义变得不清晰。

## Decision

将控制面与观测面定义为两套不同契约。

- `PolicyEngine` 是唯一同步决策面，负责对能力调用、模型请求、approval 等作出 `Allow`、`Deny`、`Ask` 的裁决。
- 事件流由 `AgentEvent` / `StorageEvent` 表达，是异步观测面，只反映运行时事实，不直接改变执行结果。
- 审批通过专门的 `ApprovalBroker` / policy broker 完成；事件层只镜像审批事实，不承担审批请求与响应通道。
- 持久化事件与实时观测可以采用不同模型，通过显式投影关联，而不是强制共用同一事件契约。

## Consequences

- UI 和 transport 不再被误当成执行仲裁者。
- 权限、审批和事件观测具有清晰分离的控制面边界。
- durable event log 与实时 event stream 可以独立演进。
- runtime 需要显式维护 policy、approval 和事件投影之间的协作关系。
