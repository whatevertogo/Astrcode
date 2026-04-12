# Astrcode 架构上下文提示词

## 项目架构方向

本项目采用 **Session Actor + Typed Kernel + Thin Application + Honest Adapters** 架构。`runtime` 这个词已从架构中消失。

## Crate 依赖关系

```
core
 ↑
 ├── kernel
 ├── session-actor（依赖 kernel）
 ├── adapter-storage / adapter-llm / adapter-tools / adapter-mcp / adapter-prompt
        ↑
    application（只依赖 core + kernel + session-actor，绝不依赖 adapter-*）
        ↑
 ├── adapter-http（组合根）
 └── adapter-tauri（组合根）

protocol（纯 DTO，依赖 core，不反向）
```

## 各层职责一句话总结

- **core**：领域类型 + 分域 trait 契约，不放任何实现
- **protocol**：对外 DTO，依赖 core，`pub use` core 类型仅为协议便利
- **kernel**：分域 typed registry + session registry + 路由，不做业务决策
- **session-actor**：每个会话是独立 tokio task，通过 kernel 调能力，只发射事件不写 storage
- **application**：薄用例入口（create_session / run_turn / spawn_agent 等），不知道任何 adapter 具体类型
- **adapter-\***：具体实现，诚实依赖，只出现在组合根和 handler 之外

## 核心设计决策

### 1. 能力路由：分域强类型，不用总枚举

```rust
// ❌ 禁止：总枚举会膨胀成弱类型总线
enum CapabilityRequest { InvokeTool { .. }, BuildPrompt { .. }, CallLlm { .. } }

// ✅ 正确：分域 trait
pub trait ToolProvider: Send + Sync { .. }
pub trait LlmProvider: Send + Sync { .. }
pub trait PromptProvider: Send + Sync { .. }
pub trait ResourceProvider: Send + Sync { .. }
```

### 2. Session 并发：Actor 模型

```rust
// session 天然是 actor：独立状态、串行消息、可取消、可观察
enum SessionMessage {
    RunTurn  { input: TurnInput, reply: oneshot::Sender<TurnStream> },
    Cancel   { turn_id: TurnId },
    Observe  { reply: oneshot::Sender<AgentEventStream> },
    Shutdown,
}
// actor 通过 kernel: Arc<Kernel> 调能力，不直接持有 provider
// in-flight 追踪天然在 actor 内部，不需要专门设计
```

### 3. 事件持久化：发射与存储解耦

```rust
// ❌ actor 直接写 storage
self.storage.append(event).await;

// ✅ actor 只发射，storage adapter 订阅
self.event_tx.send(AgentEvent::TurnStarted { .. });
```

### 4. 组合根：只在 bootstrap，平坦注册

```rust
// adapter-http/src/bootstrap.rs —— 唯一知道具体类型的地方
let kernel = Arc::new(Kernel::new());
kernel.add_tool_provider(Arc::new(BuiltinToolAdapter::new()));
kernel.add_tool_provider(Arc::new(mcp.as_tool_provider()));
kernel.add_llm_provider(Arc::new(LlmAdapter::new(config.llm)));
// handler 层只调 application，不调 kernel
```

### 5. adapter-mcp 内部两层

```
adapter-mcp/src/
├── transport/ + manager/   # 连接、JSON-RPC、reconnect、hot_reload
└── bridge/                 # 映射到 kernel 的分域 provider（impl ToolProvider 等）
```

## 硬规则（编码时必须遵守）

1. `application` 绝不 `use adapter_*`
2. `kernel` 绝不做业务决策，绝不 `use adapter_*`
3. `session-actor` 通过 `kernel` 调能力，不直接持有 provider，不直接写 storage
4. `core` 只放稳定领域契约，不放产品策略（热加载、UI 策略等）
5. handler / tauri command 层只通过 `application` 访问，不裸连 kernel
6. 能力分域强类型，不引入总枚举总线
7. 单文件不超过 800 行
8. 所有异步操作有取消机制，不在持锁状态下 await
9. 关键操作有结构化日志，错误不静默忽略

## 已知风险点（实现前需设计清楚）

| 风险 | 应对 |
|------|------|
| in-flight 请求追踪 | request id registry + cancel token + drain state，单独设计 |
| 热加载竞态 | debounce + reload 串行化 + reload 期间旧连接处理 |
| kernel 可变性 | tool/resource registry 用 DashMap，llm/prompt 启动后不变 |
| approval 跨 crate | 宿主通过 `ApprovalStore` trait 注入，不直接读 settings 实现 |
| registry 命名冲突 | 提前定：命名格式 `mcp__{server}__{tool}`，优先级 builtin < mcp < plugin |

## 架构腐化预警信号

- `use adapter_*` 出现在 application/ 或 kernel/
- handler 直接 use kernel 内部类型
- session-actor 直接持有 storage
- 总枚举 Request/Response 出现在 kernel
- kernel 出现 `if config.feature_x` 业务判断
- 单文件超过 800 行

## 渐进迁移顺序

1. 改名：`runtime-mcp` → `adapter-mcp`，消除心智混乱
2. 抽 `session-actor`：固定并发边界
3. 抽 `kernel`：立住分域 registry
4. 瘦 `runtime` → `application`：只保留用例 API，验证不依赖 adapter-*
