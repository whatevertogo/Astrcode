## ADDED Requirements

### Requirement: `application` 提供唯一业务入口 `App`

`application` crate SHALL 提供 `App` 作为 server 的唯一业务入口。

#### Scenario: server handler 只依赖 App 或其稳定服务接口

- **WHEN** 检查 `server` handler
- **THEN** handler 只依赖 `App`（或 `application` 暴露的服务 trait）
- **AND** 不直接持有 `Kernel`、`SessionRuntime`、adapter 或旧 `RuntimeService`

---

### Requirement: `application` 只依赖核心运行时层

`application` SHALL 只依赖：

- `core`
- `kernel`
- `session-runtime`

#### Scenario: application 不反向依赖边界层或实现层

- **WHEN** 检查 `application/Cargo.toml`
- **THEN** 不包含 `adapter-*`、`protocol`、`server`、`runtime*`

---

### Requirement: `application` 负责用例编排、参数校验和权限前置

`application` SHALL 负责：

- 参数校验
- 权限前置检查
- 用例编排
- 业务错误归类

#### Scenario: 非法请求在 application 层被拒绝

- **WHEN** 传入无效 session id 或非法参数
- **THEN** `application` 直接返回业务错误
- **AND** 不将错误请求继续下推到 `kernel` 或 `session-runtime`

---

### Requirement: `application` 重建治理与运行时监督模型

`RuntimeGovernance`、`RuntimeCoordinator`、`RuntimeHandle` 的职责 SHALL 迁移到 `application`，形成新的治理模型（例如 `AppGovernance`、`AppCoordinator`、`AppHandle`）。

新的治理模型 SHALL 负责：

- 托管组件生命周期
- active plugins / capabilities 快照
- reload 结果
- shutdown 协调

#### Scenario: server 状态接口不再依赖 runtime 治理类型

- **WHEN** 检查 `server` 状态接口与 mapper
- **THEN** 使用 `application` 暴露的治理快照类型
- **AND** 不再依赖 `RuntimeGovernance` / `RuntimeCoordinator` / `RuntimeHandle`

---

### Requirement: `application` 暴露 typed error，不暴露 transport concern

`application` SHALL 定义业务错误类型（如 `ApplicationError`），错误定义 SHALL NOT 混入 HTTP 状态码、Axum 类型或其他 transport 细节。

#### Scenario: HTTP 映射只在 server 层

- **WHEN** 检查错误处理链路
- **THEN** `application` 返回业务错误
- **AND** HTTP 状态码映射只发生在 `server`

---

### Requirement: `application` 不持有底层真相与 provider 实现

`application` SHALL NOT 持有 session registry、provider 实例或 transport concern。

#### Scenario: App 字段保持干净

- **WHEN** 检查 `App` 结构体
- **THEN** 只持有 `Kernel` 和 `SessionRuntime` 等核心协作者
- **AND** 不直接持有 `EventStore`、`LlmProvider`、`ToolProvider`、`PromptProvider`

#### Scenario: App 不再保存 session shadow state

- **WHEN** 检查 `App` 的字段与方法实现
- **THEN** 不存在 `HashMap<String, SessionEntry>` 一类的会话真相缓存
- **AND** session create/list/history/replay/submit 都委托给 `SessionRuntime`

---

### Requirement: `runtime/service/*` 用例逻辑迁入 `application`

`runtime/service/*` 的用例编排逻辑 SHALL 迁入 `application` 对应模块，包括：

- `config/*`
- `composer/*`
- `lifecycle/*`
- `watch/*`
- `mcp/*`
- `observability/*`
- `service_contract.rs`（重建为 `application/errors.rs` + 服务契约）

#### Scenario: runtime 不再作为用例门面

- **WHEN** 清理阶段完成
- **THEN** 旧 `runtime` crate 不再承担用例入口

---

### Requirement: 配置模型与配置 IO 分层

稳定配置结构 SHALL 位于 `core/config`；配置读取、保存、路径解析、默认值策略、环境变量解析、校验 SHALL 位于 `application/config`。

#### Scenario: core 只保留稳定配置类型

- **WHEN** 检查 `core`
- **THEN** 仅包含配置模型和纯语义类型
- **AND** 不包含文件系统路径解析或默认值策略

#### Scenario: application 负责配置 IO

- **WHEN** server 需要加载或保存配置
- **THEN** 通过 `application/config` 完成
