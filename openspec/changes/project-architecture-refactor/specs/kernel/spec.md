## ADDED Requirements

### Requirement: `kernel` 是唯一全局控制面

`kernel` crate SHALL 负责跨 session 的全局协调，包括：

- capability registry
- tool / llm / prompt / resource gateway
- surface 管理
- agent tree 监督
- 全局事件总线

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

### Requirement: `CapabilityRegistry` 迁入 `kernel/registry`

`runtime-registry` 的 `CapabilityRouter`、`ToolCapabilityInvoker` 与组装逻辑 SHALL 迁入 `kernel/registry`。

#### Scenario: kernel 提供统一能力查询与调用入口

- **WHEN** `session-runtime` 需要查询能力定义或调用能力
- **THEN** 通过 `kernel` 公共 API 完成

#### Scenario: 旧 runtime-registry 最终删除

- **WHEN** 清理阶段完成
- **THEN** workspace 不再包含 `runtime-registry`

---

### Requirement: `AgentControl` 迁入 `kernel/agent_tree`

`runtime-agent-control` SHALL 迁入 `kernel/agent_tree`，负责 lineage、subtree cancel/terminate、深度和并发约束。

#### Scenario: agent_tree 不依赖 runtime-config

- **WHEN** 检查 `kernel/agent_tree`
- **THEN** 不存在对 `astrcode_runtime_config` 的依赖

#### Scenario: 外部通过稳定 API 操作 agent_tree

- **WHEN** `session-runtime` 需要取消或终止子树
- **THEN** 通过 `Kernel::cancel_subtree()` 或 `Kernel::terminate_subtree()` 调用
- **AND** 不直接访问内部树结构

---

### Requirement: `kernel/surface` 仅承载全局 surface 协调

`runtime/service/loop_surface/*` 中的全局 surface 状态和刷新协调 SHALL 迁入 `kernel/surface`。

#### Scenario: build_agent_loop 不进入 kernel

- **WHEN** 重构完成后检查 `kernel` 模块
- **THEN** `build_agent_loop`、`LoopRuntimeDeps` 不位于 `kernel`
- **AND** 这些会话执行构造逻辑位于 `session-runtime`

#### Scenario: application 通过 kernel 触发 surface 刷新

- **WHEN** 配置或 MCP 声明变更需要刷新 surface
- **THEN** `application` 通过 `kernel.refresh_surface()`（或等价 API）触发

---

### Requirement: `kernel` 提供统一 gateway API

`kernel` SHALL 对外暴露稳定调度入口：

- `invoke_tool`
- `call_llm`
- `build_prompt`
- `read_resource`

#### Scenario: SessionActor 不直接持有 provider

- **WHEN** `session-runtime` 执行 turn
- **THEN** 通过 `kernel` 间接调用 tool/llm/prompt/resource provider
- **AND** SessionActor 字段不直接持有这些 provider

---

### Requirement: `Kernel` 公共面最小化并使用 typed error

`Kernel` 公共 API SHALL 只暴露能力，不暴露内部容器和同步原语；错误 SHALL 使用 `KernelError` 等强类型。

#### Scenario: Kernel 不暴露内部并发细节

- **WHEN** 检查 `Kernel` 及相关公共类型
- **THEN** 不存在 `pub` 的 `HashMap`、`DashMap`、`Mutex`、`RwLock`、`broadcast::Sender` 字段

#### Scenario: kernel 公共方法不返回 anyhow

- **WHEN** 检查 `kernel` 的公共方法签名
- **THEN** 返回 `KernelError`（或等价 typed error）
- **AND** 公共 API 不返回 `anyhow::Error`
