## ADDED Requirements

### Requirement: `application` 提供唯一业务入口 `App`

`application` crate SHALL 提供 `App` 作为 server 的唯一业务入口。

#### Scenario: server 只通过 App 调业务

- **WHEN** 检查 `server` 的 handler
- **THEN** handler 只依赖 `App`
- **AND** 不直接持有 `Kernel`、`SessionRuntime`、adapter 或旧 `RuntimeService`

---

### Requirement: `application` 只依赖核心运行时层

`application` SHALL 只依赖：

- `core`
- `kernel`
- `session-runtime`

#### Scenario: application 不依赖 adapter 或 protocol

- **WHEN** 检查 `application/Cargo.toml`
- **THEN** 不包含 `adapter-*`、`protocol`、`server`

---

### Requirement: `application` 负责参数校验与业务编排

`application` SHALL 负责：

- 参数校验
- 权限前置检查
- 用例编排
- 业务错误归类

#### Scenario: 非法请求在 application 被拒绝

- **WHEN** 传入无效 session id 或无效参数
- **THEN** application 直接返回业务错误
- **AND** 不把错误请求继续下推到 `kernel` 或 `session-runtime`

---

### Requirement: `application` 暴露 typed error，不暴露 transport concern

`application` SHALL 定义自己的业务错误类型，并且错误定义中 SHALL NOT 混入 HTTP 状态码、Axum 类型或其他 transport 细节。

#### Scenario: application 错误与 HTTP 解耦

- **WHEN** 检查 `application` 的错误类型
- **THEN** 它描述的是业务失败原因
- **AND** HTTP 状态码映射留在 `server`

---

### Requirement: `application` 不拥有底层真相

`application` SHALL NOT 持有 session registry、provider 实例或 transport concern。

#### Scenario: App 字段保持干净

- **WHEN** 检查 `App` 结构体
- **THEN** 它只持有 `Kernel` 和 `SessionRuntime` 之类的核心协作者
- **AND** 不直接持有 `EventStore`、`LlmProvider`、`ToolProvider`、`PromptProvider`

---

### Requirement: `runtime/service/*` 的用例编排逻辑迁入 `application`

当前 `runtime/service/` 下的用例编排逻辑 SHALL 迁入 `application` 的对应模块，例如：

- `config/*`
- `composer/*`
- `lifecycle/*`
- `watch/*`
- `mcp/*`
- `observability/*`
- `service_contract.rs`

#### Scenario: runtime 不再是用例门面

- **WHEN** 清理阶段完成
- **THEN** 旧 `runtime` crate 不再承担用例编排入口

---

### Requirement: 配置结构与配置读写分离

跨层共享且稳定的配置结构定义 SHALL 进入 `core`；配置读取、保存、路径解析、默认值策略、校验 SHALL 进入 `application/config`。

#### Scenario: core 只保留稳定配置类型

- **WHEN** 检查 `core`
- **THEN** 只包含稳定配置模型
- **AND** 不包含文件系统路径解析或默认值策略

#### Scenario: application 负责配置 IO

- **WHEN** server 需要加载配置
- **THEN** 通过 `application/config` 完成
