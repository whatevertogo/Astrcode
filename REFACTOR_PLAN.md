# AstrCode 架构重构实施计划

> 状态：Draft
> 策略：一次性大重构 + 删除 agent crate
> 顺序：Core → Runtime → Protocol → Plugin → SDK

---

## 1. 目标架构

```
crates/
├── core/           # 平台内核 (扩展)
├── protocol/       # 协议层 (新建)
├── runtime/        # Agent Runtime (新建)
├── plugin/         # Plugin Runtime (新建)
├── sdk/            # SDK (新建)
├── tools/          # 内置工具 (保留)
└── server/         # HTTP/SSE 入口 (保留，瘦身)
```

### 依赖关系

```
sdk ──────► protocol (轻量依赖)
    │
plugin ────► core ◄──── runtime
    │           │
    └───────────┼───────────┐
                │           │
              tools ◄──── server
```

---

## 2. Phase 1: 扩展 Core Kernel

### 2.1 目标结构

```
core/src/
├── lib.rs
├── error.rs              # AstrError (已有)
├── cancel.rs             # CancelToken (已有)
├── tool.rs               # Tool trait (已有)
├── action.rs             # LlmMessage, ToolCallRequest 等 (已有)
│
├── event/                # 事件子系统 (新建)
│   ├── mod.rs
│   ├── types.rs          # StorageEvent, StoredEvent, AgentEvent
│   ├── store.rs          # EventStore (JSONL 持久化)
│   └── translate.rs      # EventTranslator (StorageEvent → AgentEvent)
│
├── session/              # 会话子系统 (新建)
│   ├── mod.rs
│   ├── types.rs          # SessionMeta, SessionState
│   ├── manager.rs        # SessionManager trait + 实现
│   └── writer.rs         # SessionWriter (事件追加)
│
├── projection/           # 投影子系统 (新建)
│   ├── mod.rs
│   └── agent_state.rs    # AgentState, project()
│
├── registry/             # 注册表子系统 (新建)
│   ├── mod.rs
│   ├── tool.rs           # ToolRegistry (从 agent 移入)
│   └── capability.rs     # CapabilityRegistry (扩展现有)
│
├── policy/               # 策略引擎 (新建骨架)
│   ├── mod.rs
│   └── engine.rs         # PolicyEngine trait + 骨架实现
│
├── plugin/               # 插件注册 (新建骨架)
│   ├── mod.rs
│   ├── registry.rs       # PluginRegistry
│   └── manifest.rs       # PluginManifest (从现有移动)
│
└── runtime/              # Runtime 协调 (新建骨架)
    ├── mod.rs
    ├── traits.rs         # Orchestrator trait (已有，移动)
    └── coordinator.rs    # RuntimeCoordinator
```

### 2.2 迁移清单

| 来源 | 目标 | 内容 |
|------|------|------|
| `agent/src/events.rs` | `core/src/event/types.rs` | StorageEvent, StoredEvent |
| `core/src/event.rs` | `core/src/event/types.rs` | AgentEvent, Phase (合并) |
| `agent/src/event_log/` | `core/src/event/store.rs` | EventLog, append/load |
| `agent/src/service/replay.rs` EventTranslator | `core/src/event/translate.rs` | 事件转换逻辑 |
| `agent/src/projection.rs` | `core/src/projection/agent_state.rs` | AgentState, project() |
| `agent/src/tool_registry.rs` | `core/src/registry/tool.rs` | ToolRegistry |
| `core/src/capability.rs` | `core/src/registry/capability.rs` | CapabilityDescriptor |
| `core/src/plugin.rs` | `core/src/plugin/manifest.rs` | PluginManifest, PluginType |
| `agent/src/service/session_state.rs` | `core/src/session/` | SessionState, SessionWriter |
| `agent/src/service/types.rs` | `core/src/session/types.rs` | SessionMessage, SessionEventRecord |
| `core/src/orchestrator.rs` | `core/src/runtime/traits.rs` | Orchestrator, TurnContext |
| `core/src/kernel_api.rs` | `core/src/runtime/traits.rs` | KernelApi |

### 2.3 新建骨架

```rust
// core/src/policy/engine.rs
pub trait PolicyEngine: Send + Sync {
    fn check_tool_call(&self, tool: &str, args: &Value) -> PolicyDecision;
    fn check_capability(&self, capability: &str) -> PolicyDecision;
}

pub struct PolicyDecision {
    pub allowed: bool,
    pub reason: Option<String>,
}

// core/src/plugin/registry.rs
pub struct PluginRegistry {
    plugins: HashMap<String, PluginEntry>,
}

pub struct PluginEntry {
    pub manifest: PluginManifest,
    pub state: PluginState,
}

// core/src/runtime/coordinator.rs
pub struct RuntimeCoordinator {
    active_runtime: Arc<dyn Orchestrator>,
    registry: Arc<PluginRegistry>,
}
```

### 2.4 Cargo.toml 更新

```toml
[package]
name = "astrcode-core"
version = "0.2.0"  # 版本升级

[dependencies]
async-trait.workspace = true
chrono.workspace = true      # 新增
dashmap.workspace = true     # 新增 (从 agent 移入)
dirs.workspace = true
log.workspace = true         # 新增
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tokio.workspace = true       # 新增 (移入后需要)
uuid.workspace = true        # 新增
toml = "0.8"
```

---

## 3. Phase 2: 建立 Runtime 层

### 3.1 目标结构

```
runtime/src/
├── lib.rs
├── agent_loop.rs          # AgentLoop (从 agent 移入)
├── llm/                   # LLM 提供者 (从 agent 移入)
│   ├── mod.rs
│   ├── types.rs           # LlmRequest, LlmOutput, LlmEvent
│   ├── provider.rs        # LlmProvider trait
│   ├── anthropic.rs
│   └── openai.rs
├── prompt/                # Prompt 系统 (从 agent 移入)
│   ├── mod.rs
│   ├── block.rs
│   ├── composer.rs
│   ├── context.rs
│   ├── contribution.rs
│   ├── contributor.rs
│   ├── contributors/
│   ├── diagnostics.rs
│   ├── plan.rs
│   └── template.rs
├── turn_runner.rs         # 单轮执行 (从 agent 移入)
└── config.rs              # Config, Profile (从 agent 移入)
```

### 3.2 迁移清单

| 来源 | 目标 | 内容 |
|------|------|------|
| `agent/src/agent_loop/` | `runtime/src/agent_loop.rs` | AgentLoop, turn_runner |
| `agent/src/llm/` | `runtime/src/llm/` | LlmProvider, anthropic, openai |
| `agent/src/prompt/` | `runtime/src/prompt/` | PromptComposer 全部 |
| `agent/src/config.rs` | `runtime/src/config.rs` | Config, Profile |
| `agent/src/provider_factory.rs` | `runtime/src/provider_factory.rs` | ProviderFactory |
| `agent/src/cancel.rs` | 删除 | 使用 core::CancelToken |

### 3.3 依赖关系

```toml
[package]
name = "astrcode-runtime"
version = "0.1.0"

[dependencies]
astrcode-core = { path = "../core" }
async-trait.workspace = true
chrono.workspace = true
log.workspace = true
regex.workspace = true
reqwest.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tokio.workspace = true
uuid.workspace = true
```

---

## 4. Phase 3: 建立 Protocol 层

### 4.1 目标结构

```
protocol/src/
├── lib.rs
├── http/                  # HTTP/SSE DTO
│   ├── mod.rs
│   ├── session.rs         # SessionListItem, SessionMessageDto
│   ├── config.rs          # ConfigView, ProfileView
│   ├── model.rs           # ModelOptionDto, CurrentModelInfoDto
│   └── auth.rs            # AuthExchangeRequest/Response
├── plugin/                # 插件协议
│   ├── mod.rs
│   ├── messages.rs        # Initialize, Invoke, Result, Event, Cancel
│   ├── handshake.rs       # 握手流程定义
│   └── error.rs           # ProtocolError
└── transport/             # 传输层抽象
    ├── mod.rs
    ├── traits.rs          # Transport trait
    └── stdio.rs           # StdioTransport (骨架)
```

### 4.2 迁移清单

| 来源 | 目标 | 内容 |
|------|------|------|
| `server/src/dto.rs` | `protocol/src/http/` | 全部 DTO |
| 新建 | `protocol/src/plugin/messages.rs` | 插件协议消息 |

### 4.3 插件协议消息定义

```rust
// protocol/src/plugin/messages.rs
#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PluginMessage {
    Initialize(InitializeRequest),
    InitializeResult(InitializeResult),
    Invoke(InvokeRequest),
    Result(InvokeResult),
    Event(StreamEvent),
    Cancel(CancelRequest),
}

#[derive(Serialize, Deserialize)]
pub struct InitializeRequest {
    pub protocol_version: String,
    pub client_info: ClientInfo,
}

#[derive(Serialize, Deserialize)]
pub struct InitializeResult {
    pub server_info: ServerInfo,
    pub capabilities: Vec<CapabilityDescriptor>,
}

#[derive(Serialize, Deserialize)]
pub struct InvokeRequest {
    pub request_id: String,
    pub capability: String,
    pub payload: Value,
}

#[derive(Serialize, Deserialize)]
pub struct InvokeResult {
    pub request_id: String,
    pub outcome: InvokeOutcome,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "status")]
pub enum InvokeOutcome {
    Success { output: Value },
    Error { code: String, message: String },
}
```

### 4.4 依赖关系

```toml
[package]
name = "astrcode-protocol"
version = "0.1.0"

[dependencies]
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
# 注意：不依赖 core，保持协议层纯净
```

---

## 5. Phase 4: 建立 Plugin 层

### 5.1 目标结构

```
plugin/src/
├── lib.rs
├── loader.rs              # 插件发现与加载
├── process.rs             # 进程管理
├── handshake.rs           # 握手实现
├── executor.rs            # 调用执行
├── lifecycle.rs           # 生命周期管理
└── transport/
    ├── mod.rs
    ├── stdio.rs           # stdio 传输实现
    └── websocket.rs       # websocket 传输实现 (骨架)
```

### 5.2 核心类型

```rust
// plugin/src/loader.rs
pub struct PluginLoader {
    search_paths: Vec<PathBuf>,
}

impl PluginLoader {
    pub fn discover(&self) -> Result<Vec<PluginManifest>>;
    pub fn load(&self, manifest: &PluginManifest) -> Result<PluginInstance>;
}

// plugin/src/process.rs
pub struct PluginProcess {
    manifest: PluginManifest,
    child: Child,
    transport: Box<dyn Transport>,
}

impl PluginProcess {
    pub async fn start(manifest: &PluginManifest) -> Result<Self>;
    pub async fn initialize(&mut self) -> Result<InitializeResult>;
    pub async fn invoke(&mut self, req: InvokeRequest) -> Result<InvokeResult>;
    pub async fn shutdown(&mut self) -> Result<()>;
}

// plugin/src/transport/stdio.rs
pub struct StdioTransport {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}
```

### 5.3 依赖关系

```toml
[package]
name = "astrcode-plugin"
version = "0.1.0"

[dependencies]
astrcode-core = { path = "../core" }
astrcode-protocol = { path = "../protocol" }
async-trait.workspace = true
log.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tokio.workspace = true
```

---

## 6. Phase 5: 建立 SDK 层

### 6.1 目标结构

```
sdk/src/
├── lib.rs
├── tool.rs                # Tool 开发辅助
├── agent.rs               # Agent 开发辅助
├── context.rs             # Context Provider 开发
├── memory.rs              # Memory Provider 开发
├── hook.rs                # Policy Hook 开发
└── macros.rs              # 声明式注册宏
```

### 6.2 核心 API

```rust
// sdk/src/tool.rs
pub trait ToolHandler: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> Value;
    fn execute(&self, input: Value) -> impl Future<Output = Result<Value>>;
}

// sdk/src/macros.rs
#[macro_export]
macro_rules! declare_tool {
    ($handler:ty) => {
        #[no_mangle]
        pub extern "C" fn _astrcode_plugin_entry() -> *mut std::ffi::c_void {
            // 插件入口
        }
    };
}
```

### 6.3 依赖关系

```toml
[package]
name = "astrcode-sdk"
version = "0.1.0"

[dependencies]
astrcode-protocol = { path = "../protocol" }
serde.workspace = true
serde_json.workspace = true
# 注意：不依赖 core，保持 SDK 轻量
```

---

## 7. Server 重构

### 7.1 变化

- 删除 `dto.rs` → 改用 `astrcode_protocol::http::*`
- 删除对 `astrcode-agent` 的依赖 → 改用 `astrcode-core` + `astrcode-runtime`
- main.rs 重构为使用新的模块

### 7.2 新的依赖

```toml
[package]
name = "astrcode-server"
version = "0.2.0"

[dependencies]
astrcode-core = { path = "../core" }
astrcode-runtime = { path = "../runtime" }
astrcode-protocol = { path = "../protocol" }
astrcode-plugin = { path = "../plugin" }  # 可选
astrcode-tools = { path = "../tools" }
# ... 其他依赖保持
```

---

## 8. 工作区更新

### 8.1 Cargo.toml

```toml
[workspace]
members = [
    "crates/core",
    "crates/runtime",      # 新增
    "crates/protocol",     # 新增
    "crates/plugin",       # 新增
    "crates/sdk",          # 新增
    "crates/tools",
    "crates/server",
    "src-tauri",
]
resolver = "2"
```

### 8.2 删除 agent crate

完成迁移后：
```bash
rm -rf crates/agent
```

---

## 9. 实施检查点

### 9.1 Phase 1 完成标准

- [ ] `cargo check -p astrcode-core` 通过
- [ ] `cargo test -p astrcode-core` 通过
- [ ] 所有迁移的模块有对应的单元测试
- [ ] core 不依赖 runtime/protocol/plugin

### 9.2 Phase 2 完成标准

- [ ] `cargo check -p astrcode-runtime` 通过
- [ ] `cargo test -p astrcode-runtime` 通过
- [ ] runtime 可以独立使用 core 的类型

### 9.3 Phase 3 完成标准

- [ ] `cargo check -p astrcode-protocol` 通过
- [ ] `cargo test -p astrcode-protocol` 通过
- [ ] protocol 不依赖任何其他内部 crate

### 9.4 Phase 4 完成标准

- [ ] `cargo check -p astrcode-plugin` 通过
- [ ] `cargo test -p astrcode-plugin` 通过
- [ ] 插件进程可以启动并完成握手

### 9.5 Phase 5 完成标准

- [ ] `cargo check -p astrcode-sdk` 通过
- [ ] `cargo test -p astrcode-sdk` 通过

### 9.6 最终验收

- [ ] `cargo check --workspace` 通过
- [ ] `cargo test --workspace` 通过
- [ ] `cargo run -p astrcode-server` 启动成功
- [ ] 前端可以正常连接和交互
- [ ] `crates/agent` 目录已删除

---

## 10. 风险与缓解

### 10.1 风险：循环依赖

**缓解**：严格遵守依赖方向，core 不依赖任何其他内部 crate。

### 10.2 风险：测试覆盖丢失

**缓解**：每个迁移的模块必须保留原有测试，迁移后立即运行验证。

### 10.3 风险：功能回退

**缓解**：Phase 完成后进行端到端验证，确保服务器可启动并响应请求。

---

## 11. 时间估算

| Phase | 预估工作量 |
|-------|-----------|
| Phase 1: Core | 中等 (大量代码移动) |
| Phase 2: Runtime | 中等 (代码移动 + 整理) |
| Phase 3: Protocol | 较低 (主要是 DTO + 新定义) |
| Phase 4: Plugin | 较高 (新实现) |
| Phase 5: SDK | 中等 (新实现) |
| 集成验证 | 中等 |

---

## 12. 附录：代码迁移映射表

### agent → core

| agent 模块 | core 目标 |
|-----------|-----------|
| `events.rs` | `event/types.rs` |
| `event_log/*.rs` | `event/store.rs` |
| `projection.rs` | `projection/agent_state.rs` |
| `tool_registry.rs` | `registry/tool.rs` |
| `service/session_state.rs` | `session/writer.rs` |
| `service/types.rs` | `session/types.rs` |
| `service/replay.rs` EventTranslator | `event/translate.rs` |

### agent → runtime

| agent 模块 | runtime 目标 |
|-----------|-------------|
| `agent_loop/*.rs` | `agent_loop.rs`, `turn_runner.rs` |
| `llm/*.rs` | `llm/` |
| `prompt/*.rs` | `prompt/` |
| `config.rs` | `config.rs` |
| `provider_factory.rs` | `provider_factory.rs` |

### server → protocol

| server 模块 | protocol 目标 |
|-------------|---------------|
| `dto.rs` | `http/` |
| 新建 | `plugin/` |
