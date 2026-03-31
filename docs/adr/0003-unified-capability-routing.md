# ADR-0003: Use Core-Owned Unified Capability Routing

- Status: Accepted
- Date: 2026-03-25
- Amended: 2026-03-30

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
- built-in tool invoker 通过 `CapabilityInvoker` 接入 `CapabilityRouter`
- `plugin` 通过 `CapabilityInvoker` 把远端 capability 接到 `CapabilityRouter`
- `runtime` 只消费 `CapabilityRouter`
- `runtime assembly` 负责装配 built-in 与 plugin invoker，`server` 只消费已装配完成的 runtime surface

### 1. Core 拥有统一 router 抽象

`CapabilityRouter` 属于 `astrcode-core`，因为它代表平台统一能力边界，而不是某个具体 runtime 或 plugin 实现细节。

### 2. Tool 通过 invoker adapter 进入统一路由

`ToolRegistry` 仍然可以作为测试或批量装配时的便利容器存在，但 router 不再直接接受 `ToolRegistry`。

统一模型是：

- `Tool -> ToolCapabilityInvoker -> CapabilityRouter`

### 3. Plugin capability 通过 invoker 进入平台统一路由

远端插件不直接暴露给 runtime。平台通过：

- `Supervisor`
- `Peer`
- `PluginCapabilityInvoker`

把远端 capability 映射为 core 级 `CapabilityInvoker`，再注册到统一 router。

### 4. Runtime 只依赖统一 capability router

Runtime 在工具循环、tool definition 暴露、模型 tool choice 生成时，统一从 `CapabilityRouter` 读取工具型 capability。

这意味着 runtime 对 built-in 和 plugin capability 一视同仁。

这里的统一依赖指的是：

- generic capability invocation 由 capability name + payload 驱动
- `CapabilityKind` 主要是路由、策略和投影元数据

当前实现里，`tool` kind 仍会被 tool-call 相关适配面读取，用于：

- 生成模型可见的 tool definitions
- 限制哪些 capability 可以走 tool execution path

这属于 adapter 视角，不应演变成第二套核心调用协议。

同时，descriptor 校验不再只依赖 builder。  
无论 descriptor 是本地直接构造还是通过插件协议解码得到，runtime 和 plugin 注册路径都会做统一校验，避免把空 kind、空 name 或无效 schema 静默带进能力路由。

### 5. Runtime assembly 负责装配，server 不负责定义能力语义

Runtime bootstrap 时：

- 创建 built-in tool invoker
- 发现并启动插件 supervisor
- 收集 plugin capability invoker
- 构建统一 `CapabilityRouter`
- 把 router 注入 `RuntimeService`

`server` 可以调用 bootstrap 入口，但不再拥有 capability 语义或装配语义本身。

### 6. Descriptor 校验在装配阶段统一执行

无论 descriptor 是通过 builder 构造、直接创建还是从插件协议解码得到，在注册到 `CapabilityRouter` 时都会经过统一校验。这确保空 kind、空 name 或无效 schema 不会静默进入路由层。

当前实现中，`Tool` trait 已提供 `capability_descriptor()` 默认实现，从 `definition()` + `capability_metadata()` 自动构建 descriptor。内置工具可通过覆写这两个方法自定义元数据，而无需在 adapter 层硬编码。

## Consequences

正面影响：

- built-in 与 plugin capability 进入同一条调用链
- runtime 可以透明消费远端插件工具
- descriptor、profile、permission hint 有了统一路由入口
- 未来接 workflow runtime 或其他 capability source 时，不需要再造第二套路由
- capability 装配点不再和 HTTP/SSE transport 层硬耦合

代价：

- core 需要维护比原先更强的一层抽象
- 远端调用失败、stream terminal 语义、取消传播都必须在 invoker 层统一收敛
- 如果 capability 名称冲突，会在 router 构建期直接暴露，而不是隐式覆盖
- 原本位于 `server` 的部分装配代码需要下沉到 runtime assembly 层
