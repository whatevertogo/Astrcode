## ADDED Requirements

### Requirement: `core` 先定义端口契约，adapter 按契约实现

`core` SHALL 定义稳定端口契约（例如 `EventStore`、`LlmProvider`、`PromptProvider`、`ResourceProvider` 及本次重构所需 provider/gateway trait），adapter SHALL 仅实现这些契约。

#### Scenario: kernel 与 adapter 通过 core 契约对接

- **WHEN** 检查 `core`、`kernel`、`adapter-*`
- **THEN** `kernel` 消费 `core` 端口 trait
- **AND** `adapter-*` 通过实现 `core` trait 提供能力

---

### Requirement: 实现层统一命名为 `adapter-*`

所有实现层 crate SHALL 统一使用 `adapter-*` 前缀命名。

#### Scenario: 旧 `runtime-*` 实现层命名被清理

- **WHEN** 重构完成
- **THEN** workspace 不再保留 `runtime-llm`、`runtime-prompt`、`runtime-mcp`、`runtime-tool-loader`、`runtime-skill-loader`、`runtime-agent-loader`

---

### Requirement: adapter 只实现端口，不持有业务真相

实现型 adapter SHALL 只负责端口实现，SHALL NOT 承载 session/turn 真相、用例编排或全局控制面状态。

#### Scenario: adapter 不承载业务真相

- **WHEN** 检查任意 `adapter-*`
- **THEN** 不存在 session registry、turn loop、业务用例编排、全局治理状态

---

### Requirement: 实现型 adapter 只依赖 `core`

以下实现型 adapter SHALL 只依赖 `core` 与第三方库：

- `adapter-storage`
- `adapter-llm`
- `adapter-prompt`
- `adapter-tools`
- `adapter-skills`
- `adapter-mcp`
- `adapter-agents`

#### Scenario: adapter 不反向依赖上层

- **WHEN** 检查上述 crate 的 `Cargo.toml`
- **THEN** 不包含 `kernel`、`session-runtime`、`application`、`server`、`runtime*`

---

### Requirement: `adapter-storage` 实现 durable storage 端口

`adapter-storage` SHALL 负责 `EventStore`、projection、recovery 读路径实现。

#### Scenario: adapter-storage 不实现业务编排

- **WHEN** 检查 `adapter-storage`
- **THEN** 只提供存储能力
- **AND** 不实现会话用例或 turn 编排

---

### Requirement: `adapter-tools` 合并工具加载与桥接定义

`runtime-tool-loader` 与 `runtime-agent-tool` SHALL 合并为 `adapter-tools`，并统一基于 `CapabilitySpec` 暴露工具能力语义。

#### Scenario: 工具定义收口到 adapter-tools

- **WHEN** 检查 `adapter-tools`
- **THEN** builtin tools 与 agent collaboration tools 位于同一 crate

#### Scenario: adapter-tools 不承载协作真相

- **WHEN** 调用 `spawn` / `send` / `observe` / `close`
- **THEN** `adapter-tools` 仅负责参数定义与桥接
- **AND** 真实执行由 `session-runtime` 完成

---

### Requirement: `adapter-agents` 只负责 agent 定义加载

`adapter-agents` SHALL 只负责 agent profile/definition 的发现、解析和注册构建。

#### Scenario: adapter-agents 不包含执行逻辑

- **WHEN** 检查 `adapter-agents`
- **THEN** 不存在 turn 执行、session 控制或 agent 运行逻辑

---

### Requirement: `src-tauri` 作为宿主适配层

`src-tauri` 视为宿主适配层（`adapter-tauri` 角色），允许依赖 `server + protocol`，但 SHALL NOT 承载业务真相。

#### Scenario: tauri 只负责宿主能力

- **WHEN** 检查 `src-tauri`
- **THEN** 仅负责 sidecar 启动、窗口控制、桌面宿主能力
- **AND** 不直接实现运行时核心业务

---

### Requirement: 环境副作用能力由 `adapter-*` 或受限 support crate 实现

凡是依赖文件系统、shell、进程探测或 durable 持久化的基础设施能力，SHALL 由 `adapter-*` 或职责受限的 support crate 提供实现，并通过稳定契约暴露给上层。

这至少包括：

- project dir 解析、working dir 归一化所需的文件系统能力
- home 目录解析
- shell / process 探测与命令执行
- tool result 与等价执行产物的 durable persist
- plugin manifest 解析

#### Scenario: side effects are implemented by adapters

- **WHEN** 检查上述能力的最终实现位置
- **THEN** 真实实现 SHALL 位于某个 `adapter-*` 或 `astrcode-support` 这类职责受限的 support crate
- **AND** `core` / `application` / `session-runtime` 只通过契约消费这些能力

#### Scenario: adapter choice may vary without moving ownership back upward

- **WHEN** 团队判断某项副作用更适合 `adapter-storage` 还是其他现有 adapter
- **THEN** 可以在 adapter 层内部调整 owner
- **AND** 该实现 ownership SHALL NOT 回流到 `core`

---

### Requirement: `astrcode-support` 或等价 durable adapter 承接工具结果持久化

tool result、压缩产物或其他需要 durable 保存的执行结果，SHALL 由 `astrcode-support`、`adapter-storage` 或等价的 durable adapter 负责最终持久化实现。

#### Scenario: tool result persistence is no longer implemented in core

- **WHEN** 检查工具结果落盘与恢复相关实现
- **THEN** durable persist 逻辑 SHALL 位于 `astrcode-support`、`adapter-storage` 或等价 durable adapter
- **AND** `core` 不再直接实现这些落盘细节

---

### Requirement: shell、home 与 manifest 解析由 adapter、support crate 或组合根 owner 提供

shell 检测、home 目录解析、plugin manifest 解析等宿主相关能力，SHALL 由 `adapter-*`、`astrcode-support` 这类职责受限的 support crate，或组合根附近的 owner 提供；`core` 最多只保留共享数据结构和契约。

#### Scenario: shell detection is not implemented in core

- **WHEN** 检查 shell family 检测、默认 shell 选择、命令存在性检查
- **THEN** 这些实现 SHALL 位于 `astrcode-support::shell`、`adapter-tools` 或等价宿主 adapter
- **AND** `core` 只保留 `ShellFamily`、`ResolvedShell` 等共享数据结构

#### Scenario: plugin manifest parsing is not implemented in core

- **WHEN** 检查 `PluginManifest` 的 TOML 解析 owner
- **THEN** 实际解析实现 SHALL 位于 adapter、application 或组合根
- **AND** `core` 只保留 manifest 数据结构定义

#### Scenario: shared host path resolution is centralized outside core

- **WHEN** 多个 crate 需要共享 Astrcode home / projects / project bucket 解析
- **THEN** 这些宿主路径 helper SHALL 位于 `astrcode-support::hostpaths` 或等价受限 support crate
- **AND** `core` 不再拥有 `dirs::home_dir()`、Astrcode 根目录拼装或 `project_dir()` 这类 owner
