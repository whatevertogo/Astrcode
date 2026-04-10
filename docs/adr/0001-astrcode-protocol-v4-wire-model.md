# ADR-0001: Freeze AstrCode Protocol V4 Wire Model

- Status: Accepted
- Date: 2026-03-25

## Context

AstrCode 的插件与 transport 协议已经演进为独立边界。为了避免不同实现对同一消息语义产生分歧，需要把 wire model 统一成稳定的交互骨架。

## Decision

冻结 AstrCode Protocol V4 的外部 wire model，作为所有插件、SDK、server 和传输实现的共识。

- 插件宿主/插件之间的顶层消息固定为 `PluginMessage::Initialize`、`PluginMessage::Invoke`、`PluginMessage::Result`、`PluginMessage::Event`、`PluginMessage::Cancel`。
- 插件协议的 `EventMessage.phase` 固定为 `EventPhase::Started`、`EventPhase::Delta`、`EventPhase::Completed`、`EventPhase::Failed`，并且事件语义通过 `event` 字段承载。
- 初始化阶段以 `ResultMessage(InitializeResultData)` 结束，不再引入独立的“initialize result”顶层类型。
- `transport` 仅负责原始 JSON/text 的收发。请求/响应匹配、流式生命周期、错误语义和业务路由应由更高层协议实现来解释。
- 所有协议错误统一在 `ErrorPayload` 中表达，至少包含 `code`、`message`、`details`、`retriable`。
- 协议兼容性以协议 fixture 为基线；任何变更都必须同步更新 ADR，将其视为 breaking change。

## Consequences

- 不同插件宿主、浏览器端、桌面端和 SDK 共享同一套消息语义。
- 插件协议继续优先通过扩展 descriptor、context 或 event taxonomy 来增加能力，而不是增添顶层消息。
- 旧实验性消息格式不再兼容；协议演进必须明确对外兼容性影响。
