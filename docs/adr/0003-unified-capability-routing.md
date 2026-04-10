# ADR-0003: Use Unified Capability Routing in Runtime

- Status: Accepted
- Date: 2026-03-25

## Context

AstrCode 曾把内置工具注册、runtime 能力调度和插件能力反向调用分散到多个实现里，导致能力路由、权限检查和 descriptor 语义无法统一。当前项目已经把绝大多数 runtime 能力路由集中在 `crates/runtime-registry`。

## Decision

统一以 capability 作为平台能力抽象，并在 runtime 侧使用统一的 `CapabilityRouter`。

- `core` 继续定义能力调用契约，例如 `CapabilityCall`、`ToolDefinition`、`PolicyContext` 等接口。
- `crates/runtime-registry` 提供统一的 `CapabilityRouter` 实现，承载内置工具与插件能力的注册与路由。
- 内置工具通过 runtime bootstrap 的能力 invoker 注册到 `CapabilityRouter`。
- 插件能力通过插件适配层映射为相同的 capability，并注册到同一个 `CapabilityRouter`。
- runtime 装配层只依赖这套统一路由实现，不区分能力来源。
- descriptor 校验、权限 hint 和调度策略在注册阶段统一执行，避免无效 descriptor 静默进入执行链路。

## Consequences

- 内置工具与插件能力进入同一条调用链，runtime 可以透明消费远端能力。
- profile、permission hint 和 descriptor 校验获得统一入口。
- 新的能力来源只需接入现有路由模型，无需再造第二套路由。
- runtime-registry 作为具体实现承载更多运行时语义，`core` 保持契约层职责。
