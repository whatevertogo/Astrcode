# ADR-0001: Freeze AstrCode Protocol V4 Wire Model

- Status: Accepted
- Date: 2026-03-25

## Context

AstrCode 已经从“临时插件消息”演进为独立的 V4 协议边界。此前的主要问题不是功能不够，而是边界不稳定：

- 初始化结果曾使用临时独立消息形态
- 流式事件语义没有稳定生命周期
- coding 场景字段容易继续被塞进顶层消息壳
- `stdio`、未来 `websocket`、以及不同语言 SDK 缺少统一兼容基线

AstrCode 当前已经明确选择“保留 v4 分层思想，但重定义 AstrCode 专用字段”，并且不再兼容更早的实验性 wire shape。

## Decision

冻结 AstrCode Protocol V4 的基础 wire model，作为平台与插件 / worker / 可替换 runtime 之间的唯一协议骨架。

### 1. 基础消息固定为五类

AstrCode Protocol V4 只定义以下五类顶层消息：

- `initialize`
- `invoke`
- `result`
- `event`
- `cancel`

除非发生协议代际升级，否则不新增第六类顶层消息。

### 2. 初始化响应统一使用 `result(kind="initialize")`

初始化响应不再使用独立消息类型。初始化成功或失败都通过：

- `type = "result"`
- `kind = "initialize"`

来表达。

### 3. 流式执行生命周期固定

`event` 消息的生命周期固定为：

- `started`
- `delta`
- `completed`
- `failed`

领域语义通过 `event` 字段表达，例如：

- `message.delta`
- `reasoning.delta`
- `artifact.patch`
- `diagnostic`

### 4. 传输层保持 raw message transport

`Transport` 只负责消息收发，不承载：

- 协议状态机
- 请求响应匹配
- 流式生命周期管理
- 业务路由

这些能力由 `Peer` 层负责。

### 5. 错误模型固定为结构化 payload

协议错误统一通过结构化 `error` 字段表达，至少包含：

- `code`
- `message`
- `details`
- `retriable`

不允许把语言运行时异常对象直接暴露为协议标准。

### 6. 兼容基线使用 fixture 驱动测试冻结

`crates/protocol/tests/fixtures/v4/*.json` 是当前 V4 wire contract 的兼容基线。

任何影响这些 fixture 的变更，都视为协议 breaking change，必须：

- 修改 ADR
- 更新 fixture
- 明确说明兼容性影响

## Consequences

正面影响：

- 平台、插件、SDK 和未来其他语言实现有稳定的消息骨架
- `stdio` 与其他传输实现可以共享同一套协议语义
- 之后新增能力时优先扩 descriptor/context/event taxonomy，而不是继续增顶层消息

代价：

- 旧的实验性消息格式不再兼容
- 未来如果想改变初始化模型或流式生命周期，需要显式升级协议，而不是悄悄修改字段

## Current Implementation Status

截至 2026-03-31，V4 协议已在以下模块落地：

- `crates/protocol/src/plugin/` — 插件协议消息类型、握手、错误模型
- `crates/protocol/tests/fixtures/v4/` — 五类顶层消息的 JSON fixture 基线
- `crates/plugin/src/peer.rs` — Peer 层协议状态机与请求响应匹配
- `crates/plugin/src/transport/stdio.rs` — stdio 传输实现
- `crates/plugin/tests/v4_stdio_e2e.rs` — V4 stdio 端到端测试
