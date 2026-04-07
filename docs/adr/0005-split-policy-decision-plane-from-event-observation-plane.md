# ADR-0005: Split Policy Decision Plane from Event Observation Plane

- Status: Accepted
- Date: 2026-03-30

## Context

AstrCode 既需要能改变执行结果的同步决策能力，也需要只观察运行时事实的异步订阅能力。若把两者混在同一套 hook 或事件机制中，审批、权限和上下文策略会与 UI、SSE、telemetry 等观察者耦合，执行控制和观测语义都会变得模糊。

## Decision

将控制面与观测面定义为两套不同契约。

- `Policy` 是唯一同步决策面，负责 `Allow`、`Deny`、`Ask` 以及与上下文策略相关的执行裁决。
- `Event` 是唯一异步观测面，只表达运行时事实，不具备改变执行结果的权力。
- 审批通过专门的 broker 完成；event 层只镜像审批事实，不承担审批请求与响应通道。
- 持久化事件与运行时观测事件允许不同，通过显式投影关联，而不是强制共用同一事件模型。

## Consequences

- UI 和 transport 不再被误当成执行仲裁者。
- 权限、审批和上下文策略获得清晰的控制面边界。
- durable event log 与实时观测可以独立演进。
- runtime 需要显式维护 policy、approval 和 event projection 之间的协作关系。
