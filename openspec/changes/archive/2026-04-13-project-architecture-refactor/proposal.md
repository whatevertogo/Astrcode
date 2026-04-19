## Why

当前仓库最难读懂的地方，不是单个模块实现，而是边界失真：

- `runtime` 同时承担组合根、全局控制面、单 session 执行面、用例门面，`RuntimeService` 横跨 session / turn / agent / config / watch / mcp / observability
- `core` 反向依赖 `protocol`，导致领域层与传输层耦合在一起
- `server` 直接持有并调用 `RuntimeService`，没有清晰的用例边界
- `CapabilityWireDescriptor` 同时扮演领域语义、执行提示、传输 DTO 三种角色，导致 prompt、plugin、router、policy、tool loop 都混在同一个类型上
- 文档里虽然已经提出了“分层”和“Server Is The Truth”，但对 `kernel`、`session-runtime`、`application` 的归属仍有交叉，尤其是会话目录和事件真相归属不够干净

项目已经明确“不需要向后兼容”，所以这次重构目标不是“低风险平滑迁移”，而是一次性建立能长期看懂、能稳定扩展的架构骨架。

**架构总原则**

1. `application` 是唯一用例边界。
2. `kernel` 是唯一全局控制面，只负责跨 session、跨能力的全局协调。
3. `session-runtime` 是唯一会话真相面，只负责单 session 生命周期与执行语义。
4. `core` 只保留领域模型、端口与不变量，绝不依赖 `protocol`。
5. `adapter-*` 只实现端口，不承载业务真相。

## What Changes

### 1. `core` 摆脱 `protocol`

- 在 `core` 中新建 `CapabilitySpec`
- `CapabilityKind`、`SideEffect`、`Stability`、`PermissionSpec`、`InvocationMode` 等能力语义全部归入 `core`
- `CapabilitySpec` 同时承载运行时真正需要的执行提示字段：
  - `profiles`
  - `invocation_mode`（替代今天的 `streaming: bool`）
  - `compact_clearable`
  - `max_result_inline_size`
- `protocol` 中的 `CapabilityWireDescriptor` 退化为传输 DTO，只负责序列化、协议兼容和插件握手
- `core`、`runtime-prompt`、`runtime-agent-loop`、`plugin`、`runtime-registry` 全部改为先消费 `core::CapabilitySpec`

### 2. 新建 `kernel` crate

`kernel` 只保留全局控制面：

- capability registry
- provider gateway（tool / llm / prompt / resource 调度入口）
- surface 管理
- agent tree 监督
- 全局事件总线

`kernel` **不再**持有 session actor 目录，也不再拥有 session 状态真相。

### 3. 新建 `session-runtime` crate

`session-runtime` 成为唯一会话真相面，统一负责：

- session actor / session state
- session 目录
- turn loop
- interrupt / replay / branch
- observe / mailbox / parent delivery
- durable event append
- session catalog 广播

也就是说，会话相关状态不再在 `runtime` 和 `kernel` 之间拆半，而是全部归口到 `session-runtime`。

### 4. 新建 `application` crate

`application` 作为唯一用例边界，负责：

- 参数校验
- 用例编排
- 权限前置检查
- 业务错误归类
- 对 `kernel` 与 `session-runtime` 的协同调用

`server` 只通过 `App` 调业务，不再直接碰运行时内部句柄。

### 5. `runtime-*` 全量重命名为 `adapter-*`

因为不考虑向后兼容，这次不保留“双名字并存”或长期兼容层，直接以最终命名为目标：

- `storage` → `adapter-storage`
- `runtime-llm` → `adapter-llm`
- `runtime-prompt` → `adapter-prompt`
- `runtime-mcp` → `adapter-mcp`
- `runtime-tool-loader` + `runtime-agent-tool` → `adapter-tools`
- `runtime-skill-loader` → `adapter-skills`
- `runtime-agent-loader` → `adapter-agents`
- `src-tauri` 视为 `adapter-tauri` 的宿主实现

### 6. `server` 收口为唯一业务边界

考虑到仓库里已经存在 `crates/server/src/bootstrap/mod.rs`，组合根不再写成含糊的“未来某个 `server/bootstrap.rs`”，而是明确落到现有 server bootstrap 模块下的新文件，例如：

- `crates/server/src/bootstrap/runtime.rs`

`main.rs` 只保留：

- 启动监听
- 构造 `AppState`
- 安装路由
- 优雅关闭

真正的 runtime 组装不再藏在 `crates/runtime/src/bootstrap.rs`。

### 7. 删除旧 `runtime` crate

最终删除：

- `runtime`
- `runtime-config`
- `runtime-session`
- `runtime-execution`
- `runtime-agent-loop`
- `runtime-agent-control`
- `runtime-registry`
- 所有被新结构取代的旧 crate

不维护长期兼容层，不保留“旧 façade 继续转调新实现”的历史债。

## Non-goals

- 不改变前端交互目标，但允许内部 API 重新分层
- 不保留旧 crate 的兼容入口
- 不保留 `CapabilityWireDescriptor` 在运行时内部的中心地位
- 不做“先别名重命名、以后再收口”的折中方案
- 不为降低迁移成本牺牲结构清晰度

## Capabilities

### New Capabilities

- `capability-semantic-model`: `core::CapabilitySpec` 成为唯一能力语义模型
- `kernel`: 全局控制面
- `session-runtime`: 单 session 真相面
- `application-use-cases`: 用例边界
- `adapter-contracts`: adapter 端口契约

### Modified Capabilities

已有能力的业务行为不追求兼容旧实现路径，而是重建归属：

- capability registration
- session lifecycle
- turn execution
- mcp integration
- persistence wiring
- prompt building
- sub-agent collaboration

## Impact

### 目标 crate 结构

```text
crates/
├── core/                  # 领域模型、端口、错误、不变量、CapabilitySpec
├── protocol/              # 传输 DTO / wire，只依赖 core
│
├── kernel/                # registry / provider gateway / surface / agent tree / event hub
├── session-runtime/       # session actor / session state / turn / replay / observe / catalog
├── application/           # 用例层
├── server/                # HTTP/SSE 边界
│
├── adapter-storage/       # EventStore / projection / recovery
├── adapter-llm/           # LlmProvider 实现
├── adapter-prompt/        # PromptProvider 实现
├── adapter-tools/         # builtin tools + agent tools
├── adapter-skills/        # skill discovery/load
├── adapter-mcp/           # MCP transport + manager + bridge
├── adapter-agents/        # agent profile/definition loader
│
├── plugin/
├── sdk/
└── src-tauri/             # adapter-tauri 宿主实现
```

### 目标依赖关系

```text
protocol -> core
kernel -> core
session-runtime -> core + kernel
application -> core + kernel + session-runtime
server -> application + protocol

adapter-storage -> core
adapter-llm -> core
adapter-prompt -> core
adapter-tools -> core
adapter-skills -> core
adapter-mcp -> core
adapter-agents -> core

src-tauri -> server + protocol
plugin -> core + protocol
sdk -> protocol
```

### 开发者可见影响

- Rust 代码阅读路径会从“沿着 `runtime` 横跳”改为“先看层，再看模块”
- 所有 capability 相关逻辑改为围绕 `core::CapabilitySpec`
- session 相关状态只在 `session-runtime` 中追踪
- `server` 不再成为 runtime 内部细节的旁路入口

### 用户可见影响

- 对外 API 目标保持稳定
- 但内部实现完全允许破坏性重构，不保证旧 crate 名称、旧 Rust API、旧模块路径继续可用

## Migration Strategy

采用“先立新边界，再整体搬迁，再删除旧层”的顺序：

1. 先把 `CapabilitySpec` 和核心分层规则定清楚
2. 把 `kernel` / `session-runtime` / `application` 的职责切干净
3. 把旧实现按新边界整体迁移
4. 由 `server` 接管新的组合根
5. 最后一次性删除旧 `runtime*`

这不是兼容式重构，而是以最终形态为目标的重建式重构。
