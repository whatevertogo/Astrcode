## Requirements

### Requirement: `kernel` 是唯一全局控制面

`kernel` crate SHALL 负责跨 session 的全局协调，包括：
- capability registry（`CapabilityRouter`）
- tool / llm / prompt / resource gateway（`KernelGateway`）
- surface 管理（`SurfaceManager`）
- agent tree 监督（`AgentControl`、`KernelAgentSurface`）
- 全局事件总线（`EventHub`）

`kernel` SHALL NOT 承担单 session 真相职责，包括：
- session 目录
- session actor 状态
- turn loop
- replay / branch / interrupt
- 单 session 事件历史

#### Scenario: kernel 仅依赖 core 语义与端口

- **WHEN** 执行 `cargo check -p astrcode-kernel`
- **THEN** 编译成功
- **AND** `kernel/Cargo.toml` 不依赖 `runtime*`、`session-runtime`、`application`

#### Scenario: kernel 不包含会话真相模块

- **WHEN** 检查 `kernel` crate 模块结构
- **THEN** 不存在 `session/`、`catalog/`、`turn/`、`replay/`、`observe/`、`actor/`

---

### Requirement: `CapabilityRouter` 位于 `kernel/registry`

`kernel/registry` 提供 `CapabilityRouter`、`CapabilityRouterBuilder` 与 `ToolCapabilityInvoker`。

`CapabilityRouter` 维护一个 `HashMap<name, Arc<dyn CapabilityInvoker>>` 注册表，
支持注册、查询、原子替换、工具子集筛选和工具调用执行。

#### Scenario: kernel 提供统一能力查询与调用入口

- **WHEN** `session-runtime` 需要查询能力定义或调用能力
- **THEN** 通过 `kernel` 的 `CapabilityRouter` 或 `KernelGateway` 完成

#### Scenario: CapabilityRouter 支持原子替换整份能力面

- **WHEN** 外部 surface（MCP/plugin）发生变化
- **THEN** 组合根通过 `CapabilityRouter::replace_invokers()` 原子替换整份注册表
- **AND** 替换后自动触发 `SurfaceManager::replace_capabilities()` 和 `KernelEvent::SurfaceRefreshed` 事件

#### Scenario: CapabilityRouter 支持工具子集筛选

- **WHEN** session 需要限制可用工具范围
- **THEN** 通过 `CapabilityRouter::subset_for_tools_checked()` 创建仅含允许工具的子路由
- **AND** 未知工具名称会返回 `Validation` 错误

#### Scenario: ToolCapabilityInvoker 桥接 Tool trait 到 CapabilityInvoker

- **WHEN** 一个 `dyn Tool` 需要注册到 `CapabilityRouter`
- **THEN** 通过 `ToolCapabilityInvoker::new()` 或 `ToolCapabilityInvoker::boxed()` 包装为 `dyn CapabilityInvoker`
- **AND** 桥接层自动转换 `ToolContext` ↔ `CapabilityContext`

---

### Requirement: `AgentControl` 位于 `kernel/agent_tree`

`kernel/agent_tree` 负责 agent 控制平面：lineage、subtree cancel/terminate、深度和并发约束。
`AgentControl` 维护一个 in-memory agent 注册表，用 `RwLock<AgentRegistryState>` 保护。

#### Scenario: agent_tree 不依赖 runtime-config

- **WHEN** 检查 `kernel/agent_tree`
- **THEN** 不存在对 `astrcode_runtime_config` 的依赖
- **AND** 限额通过 `AgentControlLimits` 显式注入

#### Scenario: AgentControl 提供完整的 agent 生命周期管理

- **WHEN** 检查 `AgentControl` 公共方法
- **THEN** 包含 `spawn`, `spawn_with_storage`, `register_root_agent`, `list`, `get`,
  `cancel`, `cancel_for_parent_turn`, `terminate_subtree`, `wait`, `resume`,
  `get_lifecycle`, `set_lifecycle`, `complete_turn`, `get_turn_outcome`,
  `set_resolved_limits`, `set_delegation`

#### Scenario: AgentControl 支持 inbox 消息传递

- **WHEN** 检查 `AgentControl` 收件箱方法
- **THEN** 包含 `push_inbox`, `drain_inbox`, `wait_for_inbox`

#### Scenario: AgentControl 支持 parent delivery queue

- **WHEN** 检查 `AgentControl` 父级交付队列方法
- **THEN** 包含 `enqueue_parent_delivery`, `checkout_parent_delivery`,
  `checkout_parent_delivery_batch`, `requeue_parent_delivery`,
  `requeue_parent_delivery_batch`, `consume_parent_delivery`,
  `consume_parent_delivery_batch`, `pending_parent_delivery_count`

#### Scenario: AgentControl 提供子树和祖先查询

- **WHEN** 检查 `AgentControl` 查询方法
- **THEN** 包含 `collect_subtree_handles`, `ancestor_chain`, `find_root_agent_for_session`

---

### Requirement: `KernelAgentSurface` 提供稳定 agent 控制面

`kernel/agent_surface` 模块暴露 `KernelAgentSurface` 结构体，作为 application 层访问
agent 控制面的稳定 API，避免直接暴露 `AgentControl` 内部树结构。

#### Scenario: KernelAgentSurface 不暴露内部树结构

- **WHEN** 检查 `KernelAgentSurface` 的公共方法
- **THEN** 所有方法返回稳定视图类型（`SubRunStatusView`、`CloseSubtreeResult`）或 `SubRunHandle`
- **AND** 不暴露 `AgentRegistryState`、`AgentEntry` 等内部类型

#### Scenario: Kernel 通过 agent() 方法暴露控制面

- **WHEN** `application` 或 `server` 需要操作 agent 树
- **THEN** 通过 `kernel.agent()` 获得 `KernelAgentSurface`
- **AND** 调用 `close_subtree()`, `spawn_independent_child()`, `deliver()` 等方法

---

### Requirement: `kernel/surface` 承载全局 surface 快照

`SurfaceManager` 维护当前 capability surface 的只读快照（`SurfaceSnapshot`），
在刷新时通过 `EventHub` 发出 `KernelEvent::SurfaceRefreshed` 事件。

#### Scenario: build_agent_loop 不进入 kernel

- **WHEN** 重构完成后检查 `kernel` 模块
- **THEN** `build_agent_loop`、`LoopRuntimeDeps` 不位于 `kernel`
- **AND** 这些会话执行构造逻辑位于 `session-runtime`

#### Scenario: application 通过 kernel 触发 surface 刷新

- **WHEN** 配置或 MCP 声明变更需要刷新 surface
- **THEN** 通过 `SurfaceManager::replace_capabilities()` 触发
- **AND** 自动发布 `KernelEvent::SurfaceRefreshed` 事件

---

### Requirement: `kernel/gateway` 提供统一 gateway API

`KernelGateway` 对外暴露稳定调度入口：
- `invoke_tool` — 工具调用（委托给 `CapabilityRouter::execute_tool`）
- `call_llm` — LLM 调用（委托给 `dyn LlmProvider`）
- `build_prompt` — 提示构建（委托给 `dyn PromptProvider`）
- `read_resource` — 资源读取（委托给 `dyn ResourceProvider`）

此外还提供 `capabilities()`, `with_capabilities()`, `subset_for_tools_checked()`,
`model_limits()`, `supports_cache_metrics()` 等辅助方法。

#### Scenario: SessionActor 不直接持有 provider

- **WHEN** `session-runtime` 执行 turn
- **THEN** 通过 `KernelGateway` 间接调用 tool/llm/prompt/resource provider
- **AND** SessionActor 字段不直接持有这些 provider

---

### Requirement: `kernel/events` 提供全局事件总线

`EventHub` 基于 `tokio::sync::broadcast` 提供进程内事件广播。
`KernelEvent` 枚举当前包含：
- `SurfaceRefreshed { capability_count: usize }`

#### Scenario: EventHub 容量可配置

- **WHEN** 通过 `KernelBuilder::with_event_bus_capacity()` 指定容量
- **THEN** `EventHub::new()` 使用指定容量创建 broadcast channel
- **AND** 默认容量为 256

---

### Requirement: `Kernel` 通过 Builder 模式组装

`Kernel` 使用 `KernelBuilder` 组装，构建时需要：
- `capabilities: CapabilityRouter`（可选，默认为空）
- `llm: Arc<dyn LlmProvider>`（必须）
- `prompt: Arc<dyn PromptProvider>`（必须）
- `resource: Arc<dyn ResourceProvider>`（必须）
- `agent_limits: AgentControlLimits`（可选，使用默认值）
- `event_bus_capacity: usize`（可选，默认 256）

`Kernel` 暴露四个子面：`gateway()`, `agent_control()`, `surface()`, `events()`，
以及便捷入口 `agent()` 返回 `KernelAgentSurface`。

#### Scenario: Kernel 公共面最小化并使用 typed error

- **WHEN** 检查 `Kernel` 及相关公共类型
- **THEN** 不存在 `pub` 的 `HashMap`、`DashMap`、`Mutex`、`RwLock`、`broadcast::Sender` 字段
- **AND** `KernelBuilder::build()` 返回 `Result<Kernel, KernelError>`

#### Scenario: kernel 公共方法不返回 anyhow

- **WHEN** 检查 `kernel` 的公共方法签名
- **THEN** 返回 `KernelError`（或等价 typed error）
- **AND** 公共 API 不返回 `anyhow::Error`

#### Scenario: KernelError 覆盖核心错误场景

- **WHEN** 检查 `KernelError` 枚举
- **THEN** 包含 `Validation(String)`, `NotFound(String)`, `Invoke(String)`
- **AND** 实现了 `From<AstrError>` 转换

---

### Requirement: Kernel Owns Global Control Surface

`kernel` MUST 作为全局控制面承接 capability router、agent tree 和稳定控制合同。

#### Scenario: Application consumes stable control API

- **WHEN** `application` 编排 root agent、subrun、observe、close 或 route 请求
- **THEN** 它 SHALL 只依赖 `kernel` 暴露的稳定控制接口
- **AND** SHALL NOT 依赖 `agent_tree` 内部节点或内部状态容器

#### Scenario: Session truth remains outside kernel

- **WHEN** 系统推进某个 session 的 turn 或查询某个 session 的事件历史
- **THEN** `kernel` SHALL NOT 直接持有该 session 的真相聚合
- **AND** 这些职责 SHALL 继续由 `session-runtime` 承担

---

### Requirement: Kernel Replaces Unified Capability Surface

`kernel` MUST 支持用统一能力面一次性替换当前 surface。

#### Scenario: Builtin, MCP and plugin capabilities share one surface

- **WHEN** 组合根收集 builtin、MCP、plugin 三类能力来源
- **THEN** `kernel` SHALL 用 `CapabilityRouter::replace_invokers()` 接收它们
- **AND** SHALL 通过 `SurfaceManager::replace_capabilities()` 同步刷新 surface 快照
- **AND** SHALL 发布 `KernelEvent::SurfaceRefreshed` 事件

#### Scenario: Partial plugin refresh is not enough

- **WHEN** plugin manager 内部状态变化但 `kernel` 未替换整份 surface
- **THEN** 系统 SHALL 视该刷新为不完整实现