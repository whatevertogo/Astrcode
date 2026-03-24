# ADR-0003: Use Core-Owned Unified Capability Routing

- Status: Accepted
- Date: 2026-03-25

## Context

AstrCode 过去的内置工具、runtime 执行能力和插件能力分别位于不同入口：

- built-in tools 由本地 `ToolRegistry` 驱动
- plugin capabilities 由插件运行时单独管理
- runtime 调度又直接面对 tool registry

这种分裂模型会带来几个问题：

- runtime 只能原生消费内置 tool，不能自然消费插件 capability
- 插件能力和内置能力没有统一命名冲突检查
- server 很难把“built-in + plugin” 视为同一类平台能力
- capability descriptor、permission hint、profile 过滤无法成为统一路由条件

AstrCode 已经明确决定：平台要以 capability 作为统一能力抽象，而不是以“本地 tool”作为唯一中心。

## Decision

冻结 AstrCode 的统一能力路由模型如下：

- `core` 拥有统一 `CapabilityRouter`
- built-in `ToolRegistry` 通过 adapter 接入 `CapabilityRouter`
- `plugin` 通过 `CapabilityInvoker` 把远端 capability 接到 `CapabilityRouter`
- `runtime` 只消费 `CapabilityRouter`
- `server` 负责装配 built-in 与 plugin invoker，但不直接实现路由语义

### 1. Core 拥有统一 router 抽象

`CapabilityRouter` 属于 `astrcode-core`，因为它代表平台统一能力边界，而不是某个具体 runtime 或 plugin 实现细节。

### 2. ToolRegistry 退化为 built-in capability source

`ToolRegistry` 仍然保留，因为内置工具仍然是重要来源；但它不再是 runtime 唯一直接依赖的执行表。

统一模型是：

- `ToolRegistry -> ToolRegistryCapabilityInvoker -> CapabilityRouter`

### 3. Plugin capability 通过 invoker 进入平台统一路由

远端插件不直接暴露给 runtime。平台通过：

- `Supervisor`
- `Peer`
- `PluginCapabilityInvoker`

把远端 capability 映射为 core 级 `CapabilityInvoker`，再注册到统一 router。

### 4. Runtime 只依赖统一 capability router

Runtime 在工具循环、tool definition 暴露、模型 tool choice 生成时，统一从 `CapabilityRouter` 读取工具型 capability。

这意味着 runtime 对 built-in 和 plugin capability 一视同仁。

### 5. Server 负责装配，不负责定义能力语义

Server 启动时：

- 创建 built-in tool registry
- 发现并启动插件 supervisor
- 收集 plugin capability invoker
- 构建统一 `CapabilityRouter`
- 把 router 注入 `RuntimeService`

Server 是装配点，不是 capability 语义定义点。

## Consequences

正面影响：

- built-in 与 plugin capability 进入同一条调用链
- runtime 可以透明消费远端插件工具
- descriptor、profile、permission hint 有了统一路由入口
- 未来接 workflow runtime 或其他 capability source 时，不需要再造第二套路由

代价：

- core 需要维护比原先更强的一层抽象
- 远端调用失败、stream terminal 语义、取消传播都必须在 invoker 层统一收敛
- 如果 capability 名称冲突，会在 router 构建期直接暴露，而不是隐式覆盖
