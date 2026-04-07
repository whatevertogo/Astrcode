# ADR-0001: Freeze AstrCode Protocol V4 Wire Model

- Status: Accepted
- Date: 2026-03-25

## Context

AstrCode 已从临时插件消息演进为独立协议边界，但此前的 wire shape 不稳定：初始化响应、流式事件、coding 场景字段和多传输兼容基线都缺少清晰约束。平台需要一套可被插件、worker、不同传输和多语言 SDK 共享的稳定消息骨架。

## Decision

冻结 AstrCode Protocol V4 的 wire model，作为平台对外通信的唯一协议骨架。

- 顶层消息固定为 `initialize`、`invoke`、`result`、`event`、`cancel` 五类；协议代际升级前不新增其他顶层消息。
- 初始化响应统一使用 `result(kind="initialize")`，不再使用独立消息类型。
- 流式执行生命周期固定为 `started`、`delta`、`completed`、`failed`；领域语义通过 `event` 字段表达。
- `Transport` 只负责原始消息收发；协议状态、请求响应匹配、流式生命周期和业务路由由上层负责。
- 协议错误统一为结构化 `error` payload，至少包含 `code`、`message`、`details`、`retriable`。
- 兼容基线以协议 fixture 冻结；变更 fixture 视为 breaking change，必须同步更新 ADR 并明确兼容性影响。

## Consequences

- 平台、插件、SDK 和不同传输实现共享同一套协议语义。
- 后续新增能力应优先扩展 descriptor、context 或 event taxonomy，而不是继续增加顶层消息。
- 旧实验性消息格式不再兼容；若要调整初始化模型或事件生命周期，需要显式协议升级。
