## Purpose

建立统一业务入口与治理边界的需求叙述基准，覆盖应用层对执行入口、权限与能力治理行为的稳定契约。
## Requirements
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
- 根代理执行与子代理执行入口编排
- 跨 session 的父子协作编排

`application` MUST NOT 继续承载以下单 session 真相细节：

- 单 session 终态投影与轮询判定
- durable input queue append 细节
- child/open session observe 快照拼装
- recoverable delivery 重放与投影细节
- conversation/tool display 的底层 transcript/replay 聚合细节

在 conversation/tool display 路径上，`application` MUST 通过 `SessionRuntime` 暴露的稳定 query/command 接口读取 authoritative read model facts，并把它们作为稳定用例结果返回给 `server`；`application` MUST NOT 再把 `SessionTranscriptSnapshot`、原始 replay receiver 或需要上层继续拼装的底层事实作为正式会话展示合同向上传递。

#### Scenario: 非法请求在 application 层被拒绝

- **WHEN** 传入无效 session id 或非法参数
- **THEN** `application` 直接返回业务错误
- **AND** 不将错误请求继续下推到 `kernel` 或 `session-runtime`

#### Scenario: submit_prompt 只触发 turn，不持有 turn 内策略

- **WHEN** `App::submit_prompt` 被调用
- **THEN** `application` 只负责校验输入、读取生效配置并调用 `SessionRuntime`
- **AND** token budget、continue nudge、turn 内 observability 不在 `application` 中实现

#### Scenario: application 承接执行入口但不持有执行真相

- **WHEN** 发起根代理执行或子代理执行
- **THEN** `application` 负责解析 profile、校验输入、编排调用
- **AND** 单 session 执行真相仍由 `session-runtime` 持有
- **AND** 全局 agent control 真相仍由 `kernel` 持有

#### Scenario: application 只通过 session-runtime 稳定接口读取单 session 细节

- **WHEN** `application` 需要判断 turn 终态、读取 observe 视图或追加 input queue durable 事件
- **THEN** 统一通过 `SessionRuntime` 暴露的稳定 query/command 入口完成
- **AND** 不直接操作 `SessionState`、event replay 细节或投影组装过程

#### Scenario: conversation 工具展示用例返回稳定 facts

- **WHEN** `server` 请求 conversation snapshot、stream catch-up 或工具展示相关会话事实
- **THEN** `application` SHALL 返回已收敛的 conversation/tool display facts
- **AND** `server` 只负责 DTO 映射、HTTP 状态码与 SSE framing
- **AND** 前端消费合同 MUST NOT 依赖 `application` 之外的额外 transcript/replay 拼装

---

### Requirement: Application Uses Stable Agent Control Contracts

`application` MUST 通过稳定控制合同编排 agent control 请求。

#### Scenario: Server delegates agent control to application

- **WHEN** server 收到 subrun status、observe、route、wake、close 请求
- **THEN** `application` SHALL 负责参数校验与错误归类
- **AND** SHALL 通过稳定控制合同调用 `kernel`

#### Scenario: Application does not depend on internal tree structures

- **WHEN** `kernel` 内部控制实现重构
- **THEN** `application` 对外行为 SHALL 保持稳定
- **AND** SHALL NOT 因内部树结构重构而被迫改写实现

---

### Requirement: Application Governs Plugin Reload

`application` MUST 通过治理入口编排完整 capability reload 流程，而不是只编排 plugin 自身刷新。

#### Scenario: Reload triggers full capability refresh

- **WHEN** 上层触发 reload
- **THEN** `application` SHALL 编排完整刷新链路
- **AND** 刷新结果 SHALL 同时覆盖 builtin、MCP、plugin 能力来源
- **AND** SHALL 以统一治理结果表达当前生效 surface

#### Scenario: Governance does not hide plugin failure

- **WHEN** plugin 发现、装载、物化或参与统一 surface 替换失败
- **THEN** `application` SHALL 暴露明确错误或治理快照结果
- **AND** SHALL NOT 静默吞掉失败
- **AND** SHALL NOT 让部分 plugin 刷新结果伪装成完整 reload 成功

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

### Requirement: application SHALL expose task display facts through stable session-runtime contracts

在 conversation snapshot、stream catch-up 或等价的 task display 场景中，`application` MUST 通过 `SessionRuntime` 的 `SessionQueries::active_task_snapshot()` 稳定 query 方法读取 authoritative task facts，并在 `terminal_control_facts()` 中将结果映射为 `TerminalControlFacts.active_tasks` 字段。`application` MUST NOT 直接扫描原始 `taskWrite` tool 事件、手写 replay 逻辑或把待上层再拼装的底层事实当成正式合同向上传递。

#### Scenario: server requests conversation facts with active tasks

- **WHEN** `server` 请求某个 session 的 conversation snapshot 或 stream catch-up，且该 session 当前存在 active tasks
- **THEN** `application` SHALL 通过 `terminal_control_facts()` 返回已收敛的 task display facts
- **AND** `server` 只负责 DTO 映射（`to_conversation_control_state_dto()` 将 `active_tasks` 映射为 `ConversationControlStateDto.activeTasks`）、HTTP 状态码与 SSE framing

#### Scenario: application does not reconstruct tasks from raw tool history

- **WHEN** `application` 需要返回某个 session 的 active-task panel facts
- **THEN** 它 SHALL 统一通过 `SessionQueries::active_task_snapshot()` 读取结果
- **AND** SHALL NOT 自行遍历原始 tool result 或重写底层 projection 规则

#### Scenario: no active tasks yields None

- **WHEN** `application` 查询 task facts，但当前 session 无 active tasks（空列表或全部 completed）
- **THEN** `TerminalControlFacts.active_tasks` SHALL 为 `None`
- **AND** `ConversationControlStateDto.activeTasks` SHALL 为 `None`
