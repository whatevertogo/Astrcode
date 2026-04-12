## ADDED Requirements

### Requirement: `kernel` 作为唯一全局控制面

`kernel` crate SHALL 作为唯一全局控制面，负责：

- capability registry
- tool / llm / prompt / resource gateway
- surface 管理
- agent tree 监督
- 全局事件总线

`kernel` SHALL NOT 负责：

- session 目录
- session actor 状态
- turn loop
- replay
- 单 session 事件真相

#### Scenario: kernel 只依赖 core

- **WHEN** 执行 `cargo check -p astrcode-kernel`
- **THEN** 编译成功
- **AND** `kernel/Cargo.toml` 只依赖 `astrcode-core` 和第三方库

#### Scenario: kernel 不包含 session 真相模块

- **WHEN** 检查 `kernel` crate 的模块结构
- **THEN** 不存在 `session/`、`catalog/`、`turn/`、`replay/`、`observe/`、`actor/` 模块

---

### Requirement: `CapabilityRegistry` 迁入 `kernel`

`runtime-registry` 的 `CapabilityRouter`、`ToolCapabilityInvoker` 及相关组装逻辑 SHALL 迁入 `kernel`，成为全局控制面的一部分。

#### Scenario: runtime-registry 最终删除

- **WHEN** 清理阶段完成
- **THEN** workspace 中不再包含 `runtime-registry`

#### Scenario: kernel 提供全局能力查询与调用入口

- **WHEN** `session-runtime` 需要调用某个 tool 或查询能力定义
- **THEN** 通过 `kernel` 暴露的公开 API 完成

---

### Requirement: `AgentControl` 迁入 `kernel/agent_tree`

`runtime-agent-control` SHALL 迁入 `kernel/agent_tree`，负责：

- agent lineage
- subtree cancel
- subtree terminate
- 深度/并发约束

#### Scenario: AgentControl 不再依赖 runtime-config

- **WHEN** 检查 `kernel/agent_tree`
- **THEN** 不存在对 `astrcode_runtime_config` 的依赖

#### Scenario: 外部不直接操作 agent_tree 内部状态

- **WHEN** `session-runtime` 需要取消或终止子树
- **THEN** 通过 `Kernel::cancel_subtree()` 或 `Kernel::terminate_subtree()` 调用

---

### Requirement: `LoopSurface` 迁入 `kernel/surface`

`runtime/service/loop_surface/*` SHALL 迁入 `kernel/surface`，由 `kernel` 负责：

- 当前 surface 快照
- agent loop 依赖面热替换
- surface 刷新协调

#### Scenario: application 通过 kernel 刷新 surface

- **WHEN** 配置或 MCP 声明变更需要刷新 surface
- **THEN** `application` 通过 `kernel.refresh_surface()` 触发

---

### Requirement: `kernel` 提供 provider gateway

`kernel` SHALL 对外暴露统一调度入口：

- `invoke_tool`
- `call_llm`
- `build_prompt`
- `read_resource`（或等价接口）

#### Scenario: session-runtime 不直接持有 provider

- **WHEN** `session-runtime` 执行 turn
- **THEN** 它通过 `kernel` 间接调用 tool / llm / prompt / resource provider
- **AND** SessionActor 字段中不直接持有这些 provider

---

### Requirement: `kernel` 提供全局事件总线

跨 session 的全局广播 SHALL 由 `kernel` 持有，例如：

- capability surface 刷新
- 全局运行时状态变化
- 需要被 server 统一订阅的全局通知

#### Scenario: kernel 只广播全局事件

- **WHEN** 检查 `kernel` 的事件模型
- **THEN** 它广播的是全局控制面事件
- **AND** 不把单 session 全量事件历史也塞进 kernel

---

### Requirement: `Kernel` 只暴露稳定接口

`Kernel` 的公开 API SHALL 只暴露能力，不暴露内部容器。

#### Scenario: Kernel 不暴露内部 map

- **WHEN** 检查 `Kernel` 的 `pub` API
- **THEN** 不存在直接返回 `HashMap`、`DashMap`、`CapabilityRegistry` 内部实现、`AgentTree` 内部状态的接口

#### Scenario: Kernel 不公开同步原语字段

- **WHEN** 检查 `Kernel` 及其公开辅助类型
- **THEN** 不存在 `pub` 的 `Mutex`、`RwLock`、`DashMap`、`broadcast::Sender` 字段

---

### Requirement: `kernel` 使用分层错误类型

`kernel` SHALL 暴露自己的 typed error，而不是把 `anyhow::Error` 作为公共 API。

#### Scenario: kernel public API 不返回 anyhow

- **WHEN** 检查 `kernel` 的公共方法签名
- **THEN** 返回值使用 `KernelError` 或等价的强类型错误
