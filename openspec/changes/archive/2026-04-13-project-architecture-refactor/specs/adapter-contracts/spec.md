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
