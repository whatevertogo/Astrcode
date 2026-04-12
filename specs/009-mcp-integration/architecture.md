# Astrcode 架构决策文档

> 状态：已定稿 | 版本：综合 Claude + GPT 多轮评审

---

## 最终架构方向

**Session Actor + Typed Kernel + Thin Application + Honest Adapters**

`runtime` 这个词从架构中消失，被三个职责明确的层替代。

---

## Crate 依赖关系

```
core
 ↑
 ├── kernel
 ├── session-actor（依赖 kernel）
 ├── adapter-storage
 ├── adapter-llm
 ├── adapter-tools
 ├── adapter-mcp
 └── adapter-prompt
        ↑
    application（依赖 kernel + session-actor，不知道 adapter 具体类型）
        ↑
 ├── adapter-http（组合根在这里）
 └── adapter-tauri（组合根在这里）

protocol（纯 DTO，依赖 core）
```

---

## Crate 结构

```
crates/
├── core               # 领域类型、事件、分域 trait 契约、不变量
├── protocol           # 对外 DTO / wire（依赖 core）
├── kernel             # 分域注册表 + session registry + 路由
├── session-actor      # SessionActor / SessionHandle / mailbox
├── application        # 薄用例层，server/tauri 的稳定入口
├── adapter-storage    # JSONL 持久化
├── adapter-llm        # LLM provider
├── adapter-tools      # 内置工具
├── adapter-mcp        # MCP transport + bridge（原 runtime-mcp）
├── adapter-prompt     # Prompt 组装
├── adapter-http       # HTTP/SSE server
└── adapter-tauri      # 桌面壳
```

---

## 各层职责

### `core`：只有类型和契约

放：
- `SessionId / TurnId / AgentId`
- `TurnInput / AgentEvent / SessionContext`
- 分域 trait：`ToolProvider / LlmProvider / PromptProvider / ResourceProvider`
- 基础错误类型
- 领域不变量

不放：
- DTO / wire 格式
- JSONL / HTTP / MCP 协议细节
- 热加载策略、UI 展示策略、宿主审批方式

```rust
// core 里的分域 trait，不是总枚举
pub trait ToolProvider: Send + Sync {
    fn tool_descriptors(&self) -> Vec<ToolDescriptor>;
    async fn invoke_tool(&self, name: &str, input: Value) -> Result<Value>;
}

pub trait LlmProvider: Send + Sync {
    async fn complete(&self, req: LlmRequest) -> Result<LlmStream>;
}

pub trait PromptProvider: Send + Sync {
    async fn build(&self, ctx: PromptContext) -> Result<BuiltPrompt>;
}

pub trait ResourceProvider: Send + Sync {
    async fn list(&self) -> Result<Vec<ResourceDescriptor>>;
    async fn read(&self, uri: &str) -> Result<ResourceContent>;
}
```

### `protocol`：依赖 core，不反向

- 依赖方向：protocol → core（不是 core → protocol）
- 可以 `pub use` core 类型，但规范必须明确：
  - core 类型的定义入口永远是 core
  - protocol 里的 `pub use` 只是对外协议兼容/便利导出
  - 新增领域类型优先放 core，协议附加 DTO 才放 protocol

### `kernel`：分域注册 + 路由，不做业务

放：
- 分域 typed registries
- session registry（open / get / remove）
- 调度 provider 的 typed API

不放：
- HTTP 请求语义
- 宿主策略、权限校验
- 大量业务流程
- create_session 的业务校验

```rust
pub struct Kernel {
    // 运行时可变（热加载需要）
    tools:     Arc<DashMap<String, Arc<dyn ToolProvider>>>,
    resources: Arc<DashMap<String, Arc<dyn ResourceProvider>>>,
    // 启动后不变
    llms:      Arc<dyn LlmProvider>,
    prompts:   Arc<dyn PromptProvider>,
    sessions:  SessionRegistry,
}

impl Kernel {
    // 分域注册，不用泛型 .register()
    pub fn add_tool_provider(&self, p: Arc<dyn ToolProvider>) { ... }
    pub fn add_llm_provider(&self, p: Arc<dyn LlmProvider>) { ... }

    // 路由时强类型，不是 dispatch(Value)
    pub async fn invoke_tool(&self, name: &str, input: Value) -> Result<Value> { ... }
    pub async fn call_llm(&self, req: LlmRequest) -> Result<LlmStream> { ... }

    // session 管理
    pub async fn open_session(&self, ctx: SessionContext) -> SessionHandle { ... }
    pub fn get_session(&self, id: SessionId) -> Option<SessionHandle> { ... }
}
```

### `session-actor`：会话即 Actor

放：
- mailbox / SessionHandle
- run_turn / cancel / observe / shutdown
- 当前 turn 生命周期
- in-flight 追踪（天然在 actor 内部）
- 本 session 事件流发射

不放：
- provider 注册
- storage 具体实现
- DTO 映射
- 全局热加载协调

```rust
enum SessionMessage {
    RunTurn  { input: TurnInput, reply: oneshot::Sender<TurnStream> },
    Cancel   { turn_id: TurnId },
    Observe  { reply: oneshot::Sender<AgentEventStream> },
    Shutdown,
}

struct SessionActor {
    kernel: Arc<Kernel>,   // 通过 kernel 调用能力，不直接持有 provider
    rx:     mpsc::Receiver<SessionMessage>,
    state:  SessionState,
    current_turn: Option<(TurnId, CancellationToken)>,
    // 事件发射，不直接写 storage
    event_tx: mpsc::Sender<AgentEvent>,
}

// 对外只暴露 handle
pub struct SessionHandle {
    tx: mpsc::Sender<SessionMessage>,
    pub id: SessionId,
}
```

**关键：actor 只发射事件，持久化在外部订阅**

```rust
// ❌ 错误：actor 自己写 storage
self.storage.append(event).await;

// ✅ 正确：actor 发射，storage adapter 订阅
self.event_tx.send(AgentEvent::TurnStarted { ... });
```

### `application`：薄，面向用例，必须存在

放：
- create_session / run_turn / cancel_turn
- observe_session / spawn_agent / send_to_agent / close_agent
- 参数校验 / 协调

不放：
- prompt 构造细节
- llm 调用细节
- mcp 连接细节
- event store 细节
- 具体 adapter 类型

```rust
// 面向用例的 API，不暴露内部机制
pub async fn create_session(kernel: &Kernel, req: CreateSessionRequest) -> Result<SessionHandle>;
pub async fn run_turn(kernel: &Kernel, id: SessionId, input: TurnInput) -> Result<TurnStream>;
pub async fn cancel_turn(kernel: &Kernel, id: SessionId) -> Result<()>;
pub async fn spawn_agent(kernel: &Kernel, req: SpawnAgentRequest) -> Result<AgentHandle>;
pub async fn observe_agent(kernel: &Kernel, handle: &AgentHandle) -> Result<AgentEventStream>;
pub async fn close_agent(kernel: &Kernel, handle: AgentHandle) -> Result<()>;
```

### `adapter-mcp`：两层内部结构

```
adapter-mcp/src/
├── transport/          # 连接、JSON-RPC、reconnect、hot_reload
│   ├── mod.rs          # McpTransport trait
│   ├── stdio.rs
│   ├── http.rs
│   ├── sse.rs
│   └── mock.rs         # cfg(test) 或 test-utils feature
├── protocol/
│   ├── client.rs       # McpClient：握手、工具调用
│   ├── types.rs        # DTO
│   └── error.rs
├── manager/
│   ├── mod.rs          # McpConnectionManager
│   ├── connection.rs   # 状态机
│   └── reconnect.rs    # 重连策略
├── config/
│   ├── loader.rs
│   ├── approval.rs     # 通过宿主注入的 trait 读写
│   └── policy.rs
└── bridge/             # 映射到 kernel 的分域 provider
    ├── tool_bridge.rs      # impl ToolProvider
    ├── prompt_bridge.rs    # impl PromptProvider
    └── resource_bridge.rs  # impl ResourceProvider
```

bridge 层只做适配，不定义能力模型。MCP 自己不定义系统核心能力模型，只负责适配进 kernel。

### 组合根：平坦，诚实

```rust
// adapter-http/src/bootstrap.rs
// 唯一知道所有具体类型的地方
let kernel = Arc::new(Kernel::new());

kernel.add_tool_provider(Arc::new(BuiltinToolAdapter::new()));
kernel.add_tool_provider(Arc::new(mcp.as_tool_provider()));
kernel.add_llm_provider(Arc::new(LlmAdapter::new(config.llm)));
kernel.add_prompt_provider(Arc::new(PromptAdapter::new()));
kernel.set_event_store(Arc::new(StorageAdapter::new(config.storage)?));
```

**handler / tauri command 层只通过 application 层访问，不直接调用 kernel。**

---

## 硬规则（宪法约束）

| # | 规则 |
|---|------|
| 1 | `application` 只能依赖 `core + kernel + session-actor`，不能依赖任何 `adapter-*` |
| 2 | `kernel` 只做注册/路由/会话目录，不做用例业务 |
| 3 | `session-actor` 通过 `kernel` 调能力，不直接持有 provider |
| 4 | `actor` 只发射事件，不直接写 storage |
| 5 | `core` 只放真正稳定的领域契约，不放产品策略 |
| 6 | 能力分域强类型，不用总枚举总线 |
| 7 | handler/command 层只通过 application 访问，不裸连 kernel |
| 8 | 单文件不超过 800 行 |
| 9 | 所有异步操作有取消机制，不在持锁状态下 await |
| 10 | 关键操作有结构化日志，错误不静默忽略 |

---

## 需要提前定义的规则

### Registry 命名空间规则（开始实现前必须明确）

- tool 名称格式：`mcp__{server}__{tool}`
- 内置 tool / mcp tool / skill 优先级：builtin < mcp < plugin
- 冲突处理规则（同名谁赢）
- descriptor 来源标记方式
- prompt / resource / tool 是否共享命名空间

### settings/approval 跨 crate 访问

```rust
// ❌ approval.rs 直接读宿主配置实现
// ✅ 宿主通过 trait 注入
pub trait ApprovalStore: Send + Sync {
    async fn get_status(&self, server_id: &str) -> ApprovalStatus;
    async fn set_status(&self, server_id: &str, status: ApprovalStatus);
}
```

---

## 已知风险点

| 风险 | 具体问题 | 应对方向 |
|------|----------|----------|
| bootstrap 改动安全边界 | 修改现有组装逻辑易破坏运行 | feature flag 保护，MCP 不可用时系统照常启动 |
| in-flight 请求追踪 | disconnect 需要追踪进行中调用 | request id registry + cancel token + drain state + close reason，单独设计 |
| 热加载竞态 | 文件监听触发异步 reload 有竞态 | debounce + async reload 串行化 + 明确 reload 期间旧连接处理 |
| kernel 可变性 | 热加载需要运行时注册新 provider | tool/resource registry 用 DashMap，llm/prompt 启动后不变 |
| kernel 再次膨胀 | kernel 容易变成新版 runtime | 每个 crate 维护拒绝清单（见下） |
| 远程传输 mock 缺失 | HTTP/SSE 传输行为无测试覆盖 | 补充远程传输 mock，与 stdio mock 分开 |

---

## 架构腐化的预警信号

| 信号 | 说明 |
|------|------|
| `use adapter_*` 出现在 application/ | application 被实现层污染 |
| `use adapter_*` 出现在 kernel/ | kernel 变成新 runtime |
| kernel 出现 `if config.feature_x` | 业务决策跑错层 |
| handler 直接 use kernel 内部类型 | 门面被绕开 |
| session-actor 直接持有 storage | 事件发射与持久化耦合 |
| 总枚举 Request/Response 出现在 kernel | 强类型退化 |
| 单文件超过 800 行 | 职责堆积 |

---

## 各 Crate 拒绝清单模板

每个 crate 的 `CLAUDE.md` 需要维护拒绝清单，防止架构腐化：

```markdown
# kernel/CLAUDE.md

## 绝对不能出现
- use adapter_*（任何 adapter crate）
- HTTP 请求/响应类型
- 宿主配置读取
- 业务校验逻辑
- 超过 3 个 if 分支的业务流程

## 如果你想加 X，先问
- 它是领域契约？→ 放 core
- 它是用例入口？→ 放 application
- 它是具体实现？→ 放对应 adapter-*
```

---

## 渐进迁移顺序

### 第一步：改名（不改实现，消除心智混乱）
- `runtime-mcp` → `adapter-mcp`
- 明显实现层的 `runtime-*` 优先去 runtime 化命名

### 第二步：抽 session-actor
- 把最值钱的并发边界固定
- SessionHandle / SessionActor / mailbox

### 第三步：抽 kernel
- 把 registry + session directory 立住
- 分域注册 API

### 第四步：瘦 runtime → application
- 保留用例 API，删掉其余
- 验证 application 不依赖任何 adapter-*

---

## 子域拆分评审

| Crate | 评价 |
|-------|------|
| session-actor | ✅ 会话状态独立，天然 actor |
| adapter-storage | ✅ 诚实 adapter |
| adapter-llm | ✅ 多 provider 支持 |
| adapter-tools | ✅ 内置工具独立 |
| adapter-mcp | ✅ 外部能力接入适配器 |
| adapter-prompt | ✅ 组装管线独立 |
| runtime-registry（原） | ⚠️ 若定义了 CapabilityRouter 等核心接口，接口归 core，实现归 kernel |
