## ADDED Requirements

### Requirement: `core` 只保留纯语义、稳定契约与无副作用算法

`core` SHALL 只承载以下内容：

- 领域语义类型、稳定 DTO、ID 与值对象
- 供 `kernel`、`session-runtime`、`application`、`adapter-*` 共享的 port trait / gateway trait
- 不依赖文件系统、shell、进程状态或单 session durable 真相的纯函数算法

`core` MUST NOT 承载以下职责：

- 单 session durable replay / projection 真相
- 全局运行时协调与关闭编排
- 文件系统 canonicalize、project dir 解析、working dir 归一化等 IO 逻辑
- shell / process 探测与命令执行
- durable tool result 落盘实现
- home 目录解析
- plugin manifest 的 TOML 解析
- 具体 HTTP 客户端错误类型绑定

#### Scenario: core remains side-effect free

- **WHEN** 检查 `crates/core/src`
- **THEN** 其中只包含纯语义模型、trait 契约与无副作用辅助逻辑
- **AND** 不存在依赖 shell 调用或文件系统读写的业务 helper
- **AND** 不存在对 home 目录解析、manifest 解析或具体 HTTP client 错误类型的 owner 语义

#### Scenario: session projection logic no longer lives in core

- **WHEN** 检查 input queue replay、turn projection snapshot 与等价的 durable projection 逻辑
- **THEN** 它们 SHALL 位于 `session-runtime`
- **AND** `core` 不再保留会话事件回放所需的 authoritative projection 实现

---

### Requirement: `core` 通过契约暴露能力，不拥有运行时 owner

`core` 可以定义稳定端口，但 MUST NOT 直接拥有会话级或进程级运行时 owner。

#### Scenario: runtime coordinator is not owned by core

- **WHEN** 检查全局关闭、状态协调或运行时生命周期 owner
- **THEN** 这些 owner SHALL 位于 `server` 组合根或等价 bootstrap 层
- **AND** `core` 最多只定义相关契约或值对象

#### Scenario: adapters implement side-effectful contracts behind core traits

- **WHEN** 某个能力需要文件系统、shell 或 durable 持久化
- **THEN** `core` 只定义调用契约
- **AND** 真实实现 SHALL 由 `adapter-*` 提供

#### Scenario: core error surface is transport-library neutral

- **WHEN** 检查 `AstrError` 与等价基础错误类型
- **THEN** 其 HTTP / 远程调用错误表达 SHALL 使用中立字段或通用 error source
- **AND** SHALL NOT 直接绑定 `reqwest::Error` 这类具体客户端库类型

---

### Requirement: `core::agent` 对外语义稳定且内部按子域拆分

`core::agent` SHALL 维持既有公共语义与导出能力，但内部实现 MUST 按职责拆分为多个子模块，而不是继续由单个膨胀的 `mod.rs` 承担全部责任。

#### Scenario: agent module is decomposed without changing semantics

- **WHEN** 检查 `crates/core/src/agent`
- **THEN** 可以按子域阅读定义、配置与共享值对象
- **AND** 外部调用方不需要依赖单个超大入口文件才能使用 `core::agent`
