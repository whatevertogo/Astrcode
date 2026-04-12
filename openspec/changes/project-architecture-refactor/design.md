## Context

当前代码已经给出三个非常明确的信号：

1. `crates/runtime/src/service/mod.rs` 中的 `RuntimeService` 持有 session、loop surface、config、observability、agent loader/profile、watch、mcp manager 等几乎所有运行时状态，它已经不是“服务门面”，而是“超级内核”。
2. `docs/architecture/crates-dependency-graph.md` 明确显示 `core -> protocol`、`server -> runtime`，说明领域层与传输层、边界层与内核层都已经反向耦合。
3. `CapabilityDescriptor` 不是单纯协议类型，它已经被 `core`、`runtime-registry`、`runtime-prompt`、`runtime-agent-loop`、`plugin` 同时消费。要做干净架构，不能只改 crate 名字，必须先重建能力模型。

本次设计不考虑向后兼容，也不以迁移成本为首要约束。目标是让 Rust 代码的阅读路径和职责边界足够清晰，能让人按层理解，而不是按历史偶然性理解。

## Goals / Non-Goals

**Goals**

- 断开 `core -> protocol`
- 建立 `core -> kernel -> session-runtime -> application -> server` 的单向分层
- 将“全局控制面”和“单 session 真相面”拆开
- 让 `CapabilitySpec` 成为运行时唯一能力语义模型
- 让 `server` 只通过 `application` 触达业务逻辑
- 删除旧 `runtime` 超级门面

**Non-Goals**

- 不保留旧 Rust API 兼容层
- 不保留旧 crate 名称或旧模块路径
- 不为了平滑迁移而继续允许混层
- 不把“先改名后收敛”当成最终方案

## Rust 落地原则

这次架构收敛除了分层，还必须符合 Rust 最佳实践，否则只是把旧复杂度换个目录名。

1. **public surface 最小化**
   复杂结构体不公开字段，只公开构造函数、handle 和稳定方法。
2. **状态所有权单点归属**
   `DashMap`、`RwLock`、`Mutex`、`CancellationToken` 这类同步原语只能留在拥有该状态的 crate 内部，不能跨层外泄。
3. **typed error 优先**
   `core`、`kernel`、`session-runtime`、`application` 分别定义清晰错误类型；`anyhow` 只允许留在程序入口和测试辅助层。
4. **typed id 优先**
   `SessionId`、`TurnId`、`AgentId`、`CapabilityName` 等使用 newtype，而不是在内部大量传 `String`。
5. **trait object 用在边界，不用来掩盖设计问题**
   `Arc<dyn Trait>` 只用于 adapter 端口和需要运行时替换的边界；内部热路径优先使用清晰的具体类型或受控抽象。
6. **构造期和运行期分离**
   Builder / bootstrap 负责组装依赖，运行期对象负责执行业务，不写“超长 new() + 超长 service struct”的 God Object。

## Decisions

### D1: `CapabilitySpec` 归 `core`，`CapabilityDescriptor` 退为 wire DTO

**决策**

`core` 新建 `CapabilitySpec`，作为运行时内部唯一能力语义模型。`protocol::CapabilityDescriptor` 只保留传输与插件握手职责。

**理由**

今天的 `CapabilityDescriptor` 混合了三类信息：

- 领域语义：`name`、`kind`、`description`、`permissions`、`side_effect`、`stability`
- 执行提示：`profiles`、`streaming`、`compact_clearable`、`max_result_inline_size`
- 传输细节：serde 注解、协议形状、插件握手载荷

如果继续把这三类职责压在一起，`core` 永远摆脱不了 `protocol`，而 `runtime-prompt`、`plugin`、`router`、`tool loop` 也会继续围着 DTO 打转。

**CapabilitySpec 结构**

```rust
pub struct CapabilitySpec {
    pub name: String,
    pub kind: CapabilityKind,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub invocation_mode: InvocationMode,
    pub concurrency_safe: bool,
    pub compact_clearable: bool,
    pub profiles: Vec<String>,
    pub tags: Vec<String>,
    pub permissions: Vec<PermissionSpec>,
    pub side_effect: SideEffect,
    pub stability: Stability,
    pub max_result_inline_size: Option<usize>,
}

pub enum InvocationMode {
    Unary,
    Streaming,
}
```

**为什么 `profiles` 和 `streaming` 进入 core**

- `profiles` 不是单纯传输字段，它影响 runtime 的路由与可见性
- `streaming` 不是序列化细节，而是调用语义，应由运行时能力模型表达

**不进入 core 的内容**

- 传输层 serde 注解
- 插件握手消息结构
- 纯协议扩展字段

如果未来仍需保留扩展信息，应在 `protocol` 侧维护，而不是继续污染 `core`。

**实现要求**

- `ToolCapabilityMetadata` 改为构建 `CapabilitySpec`
- `CapabilityInvoker` 改为返回 `CapabilitySpec`
- `runtime-prompt`、`runtime-agent-loop`、`plugin`、`runtime-registry` 全部改为消费 `CapabilitySpec`

### D2: `kernel` 只负责全局控制面

**决策**

`kernel` 只承载跨 session 的全局控制职责：

- capability registry
- tool / llm / prompt / resource gateway
- surface 管理
- agent tree 监督
- 全局事件总线

**不属于 kernel 的内容**

- session actor 目录
- session 状态真相
- turn loop
- replay
- session 事件历史

**理由**

这些内容都与“单个会话如何运行”相关，而不是“整个系统如何全局协调”相关。把 session 目录放到 kernel，只会让 kernel 重新变成一个新的 `runtime`。

**建议模块**

```text
kernel/
├── registry/
├── gateway/
├── surface/
├── agent_tree/
└── events/
```

**Kernel 核心结构**

```rust
pub struct Kernel {
    registry: CapabilityRegistry,
    surface: SurfaceManager,
    agent_tree: AgentControl,
    event_hub: EventHub,
    llm: Arc<dyn LlmProvider>,
    prompt: Arc<dyn PromptProvider>,
}
```

### D3: `session-runtime` 负责唯一会话真相

**决策**

`session-runtime` 统一持有所有 session 相关真相：

- session 目录
- session actor
- session state
- turn loop
- interrupt / replay / branch
- observe / mailbox / routing
- durable append
- session catalog 广播

**理由**

当前文档里最大的结构问题之一，是把“session 目录”写进 `kernel`，同时又把 actor/state/replay 写进 `session-runtime`。这会导致职责天然冲突。

既然我们要可读性和干净结构，就必须明确：

> 只要是“某个 session 的生命周期、状态、执行、事件”，全部归 `session-runtime`。

**建议模块**

```text
session-runtime/
├── catalog/
├── actor/
├── state/
├── turn/
├── interrupt/
├── replay/
├── observe/
└── context/
```

**SessionRuntime 核心结构**

```rust
pub struct SessionRuntime {
    sessions: DashMap<SessionId, Arc<SessionActor>>,
    event_store: Arc<dyn EventStore>,
    kernel: Arc<Kernel>,
    catalog_events: broadcast::Sender<SessionCatalogEvent>,
}
```

### D4: `application` 作为唯一用例边界

**决策**

`application` 暴露 `App`，作为 server 唯一业务入口。

```rust
pub struct App {
    kernel: Arc<Kernel>,
    session_runtime: Arc<SessionRuntime>,
}
```

**职责**

- 参数校验
- 用例编排
- 权限前置检查
- 业务错误归类
- 面向 server 的稳定返回值

**不做**

- 不持有 adapter 实例
- 不关心 HTTP 状态码
- 不实现底层 provider
- 不持有 session 真相

### D5: 组合根放入现有 server bootstrap 模块

**决策**

组合根从 `crates/runtime/src/bootstrap.rs` 迁出，放到现有 server bootstrap 模块下的新文件，例如：

- `crates/server/src/bootstrap/runtime.rs`

而不是继续让 `runtime` 负责组装自己，再由 `server` 消费这个已经装好的黑盒。

**理由**

仓库中已经存在 `crates/server/src/bootstrap/mod.rs`。继续写“未来放到 `server/bootstrap.rs`”不够严谨，还会和现有模块结构冲突。

**目标结构**

```text
crates/server/src/
├── main.rs
├── bootstrap/
│   ├── mod.rs
│   └── runtime.rs
├── http/
└── ...
```

`main.rs` 只负责启动，`bootstrap/runtime.rs` 负责组装 `AppState`。

### D6: `adapter-*` 只实现端口

**决策**

所有实现层 crate 统一为 `adapter-*`，并严格限定为“端口实现层”。

**命名映射**

```text
storage                    -> adapter-storage
runtime-llm                -> adapter-llm
runtime-prompt             -> adapter-prompt
runtime-mcp                -> adapter-mcp
runtime-tool-loader
runtime-agent-tool         -> adapter-tools
runtime-skill-loader       -> adapter-skills
runtime-agent-loader       -> adapter-agents
src-tauri                  -> adapter-tauri（宿主实现）
```

**规则**

- adapter 只实现 `core` 定义的 trait / 端口
- adapter 不拥有业务真相
- adapter 不反向依赖 `kernel`、`session-runtime`、`application`

### D7: `runtime-config` 拆散，不保留独立中心层

**决策**

`runtime-config` 不保留为独立“中间层”，按职责拆散：

- 稳定、跨层共享的配置结构定义进入 `core`
- 读取、保存、路径解析、默认值策略、环境变量解析、校验逻辑进入 `application`

**理由**

配置不是一个独立业务层，而是跨层输入。单独保留 `runtime-config` 只会让依赖继续模糊。

## Risks / Trade-offs

### R1: `CapabilityDescriptor` 退位会牵动面很广

这是必要代价，不是副作用。当前的广泛使用正说明它放错了层。

### R2: `session-runtime` 会显著变大

这是合理的，因为它本来就应该成为“会话真相面”。只要内部再按 `catalog / actor / turn / replay / observe` 分模块，它依然比今天的 `runtime` 更容易理解。

### R3: server 组合根迁出后，启动链需要重写

这也是刻意接受的变化。现状是 server 在消费一个已经被 runtime 装配好的黑盒，这本身就让边界失真。

## Migration Plan

### Phase 0: 先修文档与边界定义

- 定义 `CapabilitySpec`
- 定义 `kernel` / `session-runtime` / `application` 边界
- 明确组合根进入 `server/bootstrap/`

### Phase 1: 改能力模型

- `core` 去掉 `protocol` 依赖
- `CapabilityInvoker` 改为返回 `CapabilitySpec`
- `ToolCapabilityMetadata` 改为构建 `CapabilitySpec`
- `plugin` / `runtime-prompt` / `runtime-agent-loop` / `runtime-registry` 全部切到 `CapabilitySpec`

### Phase 2: 抽 `kernel`

- `runtime-registry` → `kernel/registry`
- provider gateway → `kernel/gateway`
- `runtime-agent-control` → `kernel/agent_tree`
- `loop_surface` → `kernel/surface`
- 全局广播 → `kernel/events`

### Phase 3: 抽 `session-runtime`

- `runtime-session` → `session-runtime/state`
- `runtime-agent-loop` → `session-runtime/turn`
- `runtime-execution` → `session-runtime/actor` + `context`
- `runtime/service/session` → `session-runtime/catalog`
- `runtime/service/turn/*` → `session-runtime/turn` + `interrupt` + `replay`
- `runtime/service/agent/*` → `session-runtime/observe`

### Phase 4: 抽 `application`

- `runtime/service/config/*` → `application/config`
- `runtime/service/composer/*` → `application/composer`
- `runtime/service/lifecycle/*` → `application/lifecycle`
- `runtime/service/watch/*` → `application/watch`
- `runtime/service/mcp/*` → `application/mcp`
- `runtime/service/observability/*` → `application/observability`
- `service_contract.rs` → `application/errors.rs`

### Phase 5: server 接管组合根

- `runtime/src/bootstrap.rs` 的组装逻辑迁入 `crates/server/src/bootstrap/runtime.rs`
- `main.rs` 只保留启动逻辑
- handler 全部改为只依赖 `App`

### Phase 6: 重命名并删除旧层

- 所有 `runtime-*` 改为 `adapter-*`
- 删除 `runtime`
- 删除被替代的旧 crate
- 更新 workspace

## Open Questions

1. `plugin` 侧是否保留一层面向协议的 `CapabilityDescriptor` builder，还是完全只从 `CapabilitySpec` 投影生成 DTO。
2. `PromptDeclaration` 最终归 `adapter-prompt` 还是沉到 `core` 作为稳定扩展块协议。当前更倾向保留在 `adapter-prompt`，避免 `core` 再次吸入渲染细节。
