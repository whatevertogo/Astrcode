# ADR-0003: Use Core-Owned Unified Capability Routing

- Status: Accepted
- Date: 2026-03-25

## Context

AstrCode 曾分别通过本地工具注册、插件运行时和 runtime 自身入口管理能力。这种分裂模型让 built-in tool、plugin capability 和 runtime 调度无法共享同一套路由、校验和命名规则，也让 server 被迫承担本不属于它的能力语义。

## Decision

统一以 capability 作为平台能力抽象，并由 `core` 拥有统一路由。

- `core` 持有统一的 `CapabilityRouter`。
- built-in tool 通过 `CapabilityInvoker` 适配后进入 `CapabilityRouter`。
- plugin capability 通过对应 invoker 映射为同一套 capability，再注册到 `CapabilityRouter`。
- `runtime` 只依赖统一 capability 路由，不区分 built-in 与 plugin 来源。
- runtime assembly 负责装配 built-in 与 plugin invoker；server 只消费已装配完成的 runtime surface。
- descriptor 校验在注册阶段统一执行，避免无效 descriptor 静默进入路由层。

## Consequences

- built-in 与 plugin capability 进入同一条调用链，runtime 可以透明消费远端能力。
- profile、permission hint 和 descriptor 校验获得统一入口。
- 后续接入新的 capability source 时无需再造第二套路由。
- core 需要承担更强的抽象职责，且调用失败、取消传播和流式终止语义必须在 invoker 层统一收敛。
