## ADDED Requirements

### Requirement: 实现层统一命名为 `adapter-*`

所有实现层 crate SHALL 统一使用 `adapter-*` 前缀命名。

#### Scenario: runtime 实现层不再保留旧命名

- **WHEN** 重构完成
- **THEN** workspace 中不再保留 `runtime-llm`、`runtime-prompt`、`runtime-mcp`、`runtime-tool-loader`、`runtime-skill-loader`、`runtime-agent-loader`

---

### Requirement: adapter 只实现端口，不持有业务真相

实现型 adapter SHALL 只实现 `core` 定义的端口，不持有业务真相。

#### Scenario: adapter 不承载 session/turn 真相

- **WHEN** 检查任意 `adapter-*`
- **THEN** 不存在 session registry、turn loop、业务用例编排、全局控制面状态

---

### Requirement: 实现型 adapter 只依赖 `core`

以下实现型 adapter SHALL 只依赖 `core` 和第三方库：

- `adapter-storage`
- `adapter-llm`
- `adapter-prompt`
- `adapter-tools`
- `adapter-skills`
- `adapter-mcp`
- `adapter-agents`

#### Scenario: adapter 不反向依赖 runtime 层

- **WHEN** 检查这些 crate 的 `Cargo.toml`
- **THEN** 不包含 `kernel`、`session-runtime`、`application`、`server`

---

### Requirement: `adapter-storage` 实现 durable storage 端口

`storage` SHALL 重命名为 `adapter-storage`，负责：

- `EventStore`
- projection
- recovery 读路径

#### Scenario: adapter-storage 不拥有业务编排

- **WHEN** 检查 `adapter-storage`
- **THEN** 它只提供存储实现
- **AND** 不实现 session 用例或 turn 编排

---

### Requirement: `adapter-tools` 合并 builtin tools 与 agent tools

`runtime-tool-loader` 与 `runtime-agent-tool` SHALL 合并为 `adapter-tools`。

#### Scenario: 工具定义收口

- **WHEN** 检查 `adapter-tools`
- **THEN** builtin tools 与 agent collaboration tools 位于同一 crate 内

#### Scenario: adapter-tools 不拥有协作真相

- **WHEN** `spawn` / `send` / `observe` / `close` 被调用
- **THEN** `adapter-tools` 只负责参数定义与桥接
- **AND** 真实执行由 `session-runtime` 完成

---

### Requirement: `adapter-agents` 只负责 agent 定义加载

`runtime-agent-loader` SHALL 重命名为 `adapter-agents`，只负责：

- agent profile/definition 发现
- 解析
- 注册表构建

#### Scenario: adapter-agents 不包含执行逻辑

- **WHEN** 检查 `adapter-agents`
- **THEN** 不存在 turn 执行、session 控制或 agent 运行逻辑

---

### Requirement: `src-tauri` 作为宿主 adapter

`src-tauri` 视为 `adapter-tauri` 的宿主实现，允许依赖 `server` + `protocol`，但不承载业务真相。

#### Scenario: Tauri 只负责宿主与窗口

- **WHEN** 检查 `src-tauri`
- **THEN** 它负责 sidecar 启动、窗口控制与桌面宿主能力
- **AND** 不直接实现运行时内核
