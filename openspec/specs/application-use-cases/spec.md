## Purpose

建立统一业务入口与治理边界的需求叙述基准，覆盖应用层对执行入口、权限与能力治理行为的稳定契约。
## Requirements
### Requirement: `application` 提供唯一业务入口 `App`

`application` crate SHALL 提供 `App` 作为 server 的唯一业务入口。

`App` 通过 port trait 与外部系统解耦，持有：
- `kernel: Arc<dyn AppKernelPort>` — kernel 控制面
- `session_runtime: Arc<dyn AppSessionPort>` — session-runtime 稳定端口
- `profiles: Arc<ProfileResolutionService>` — agent profile 解析
- `config_service: Arc<ConfigService>` — 配置服务
- `composer_service: Arc<ComposerService>` — composer 补全服务
- `composer_skills: Arc<dyn ComposerSkillPort>` — skill 查询端口
- `governance_surface: Arc<GovernanceSurfaceAssembler>` — 治理面组装器
- `mode_catalog: Arc<ModeCatalog>` — 模式目录
- `mcp_service: Arc<McpService>` — MCP 服务
- `agent_service: Arc<AgentOrchestrationService>` — agent 编排服务

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
- **THEN** `application` 直接返回 `ApplicationError::InvalidArgument` 或 `ApplicationError::PermissionDenied`
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

`application` MUST 通过 `AppGovernance` 编排完整 capability reload 流程，而不是只编排 plugin 自身刷新。

#### Scenario: Reload triggers full capability refresh

- **WHEN** 上层触发 reload
- **THEN** `application` SHALL 编排完整刷新链路（通过 `RuntimeReloader` trait）
- **AND** 刷新结果 SHALL 同时覆盖 builtin、MCP、plugin 能力来源
- **AND** SHALL 以 `ReloadResult` 表达当前生效 surface

#### Scenario: Governance does not hide plugin failure

- **WHEN** plugin 发现、装载、物化或参与统一 surface 替换失败
- **THEN** `application` SHALL 暴露明确错误或治理快照结果
- **AND** SHALL NOT 静默吞掉失败
- **AND** SHALL NOT 让部分 plugin 刷新结果伪装成完整 reload 成功

### Requirement: `application` 通过 `AppGovernance` 重建治理模型

`AppGovernance` 通过以下 trait 提供者实现治理：

- `RuntimeGovernancePort` — 运行时治理快照与关闭
- `ObservabilitySnapshotProvider` — 可观测性指标
- `SessionInfoProvider` — 会话计数与列表
- `RuntimeReloader` — 重载策略（可选）

#### Scenario: server 状态接口不再依赖 runtime 治理类型

- **WHEN** 检查 `server` 状态接口与 mapper
- **THEN** 使用 `application` 暴露的治理快照类型（`GovernanceSnapshot`, `ReloadResult` 等）
- **AND** 不再依赖 `RuntimeGovernance` / `RuntimeCoordinator` / `RuntimeHandle`

---

### Requirement: `application` 暴露 typed error，不暴露 transport concern

`application` SHALL 定义 `ApplicationError`，包含 `InvalidArgument`, `PermissionDenied`, `Conflict`, `NotFound`, `Internal` 变体。错误定义 SHALL NOT 混入 HTTP 状态码、Axum 类型或其他 transport 细节。

#### Scenario: HTTP 映射只在 server 层

- **WHEN** 检查错误处理链路
- **THEN** `application` 返回 `ApplicationError`
- **AND** HTTP 状态码映射只发生在 `server`

---

### Requirement: `application` 不持有底层真相与 provider 实现

`application` SHALL NOT 持有 session registry、provider 实例或 transport concern。

#### Scenario: App 字段保持干净

- **WHEN** 检查 `App` 结构体
- **THEN** 只通过 port trait（`AppKernelPort`, `AppSessionPort`）持有核心协作者
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

在 conversation snapshot、stream catch-up 或等价的 task display 场景中，`application` MUST 通过 `SessionRuntime` 的稳定 query 方法读取 authoritative task facts，并在 `terminal_control_facts()` 中将结果映射为 `TerminalControlFacts.active_tasks` 字段。

#### Scenario: server requests conversation facts with active tasks

- **WHEN** `server` 请求某个 session 的 conversation snapshot 或 stream catch-up，且该 session 当前存在 active tasks
- **THEN** `application` SHALL 通过 `terminal_control_facts()` 返回已收敛的 task display facts
- **AND** `server` 只负责 DTO 映射

#### Scenario: application does not reconstruct tasks from raw tool history

- **WHEN** `application` 需要返回某个 session 的 active-task panel facts
- **THEN** 它 SHALL 统一通过 `SessionQueries::active_task_snapshot()` 读取结果
- **AND** SHALL NOT 自行遍历原始 tool result 或重写底层 projection 规则

#### Scenario: no active tasks yields None

- **WHEN** `application` 查询 task facts，但当前 session 无 active tasks
- **THEN** `TerminalControlFacts.active_tasks` SHALL 为 `None`

---

### Requirement: `application` 内部按职责分块组织

`application` 内部 SHALL 按以下职责分块：

- `ports` — 与外部系统交互的 port trait（AppKernelPort, AppSessionPort, AgentKernelPort, AgentSessionPort, ComposerSkillPort）
- `execution` — 根代理与子代理执行入口编排
- `agent` — agent 编排服务（observe, wake, routing, terminal, context, collaboration flow）
- `governance_surface` — 治理面组装器与策略（assembler, policy, prompt, inherited）
- `terminal` / `terminal_queries` — 终端控制态查询
- `config` — 配置 IO（api_key, constants, env_resolver, mcp, selection, validation）
- `composer` — composer 补全服务
- `mcp` — MCP 服务
- `mode` — 模式目录与编译（catalog, compiler, validator, builtin_prompts）
- `observability` — 可观测性收集与指标快照
- `watch` — 文件监控服务
- `lifecycle` — 应用层治理模型（AppGovernance）
- `session_use_cases` — 会话级用例编排
- `agent_use_cases` — agent 级用例编排
- `session_plan` — session plan 查询
- `errors` — ApplicationError 定义

#### Scenario: port trait 实现依赖反转

- **WHEN** 检查 `ports` 模块
- **THEN** `App` 不直接依赖 `Kernel` 或 `SessionRuntime` 具体类型
- **AND** 通过 `AppKernelPort`、`AppSessionPort` 等 trait 解耦

### Requirement: `application` SHALL 在 session 提交入口编排 active workflow

`application` 在提交 session prompt 前 MUST 先解析当前 active workflow 与 current phase，再决定是否需要注入 phase overlay、解释用户信号、执行 phase 迁移，最后才编译治理面并委托 `session-runtime` 执行 turn。

#### Scenario: active workflow 为当前提交追加 phase overlay

- **WHEN** 当前 session 存在 active workflow，且当前 phase 为本轮提交生成额外 prompt declarations
- **THEN** `application` SHALL 把这些 declarations 通过现有 submission prompt path 注入
- **AND** SHALL NOT 绕过现有 governance surface / prompt assembly 标准路径

#### Scenario: 没有 active workflow 时保持现有 mode-only 提交流程

- **WHEN** 当前 session 没有 active workflow
- **THEN** `application` SHALL 继续沿用现有 mode/governance 提交流程
- **AND** SHALL NOT 要求上层调用方额外提供 workflow 参数才能完成一次普通提交

### Requirement: `application` SHALL 通过稳定 runtime 合同消费 workflow 所需事实

`application` 实现 workflow orchestration 时 MUST 通过 `session-runtime` 稳定 query / command 合同读取会话事实和推进 turn，而不是直接持有或篡改 runtime 内部状态结构。

#### Scenario: workflow approval 通过稳定入口触发 mode 迁移

- **WHEN** 某个 workflow signal 需要把 session 从一个 phase 迁移到绑定的下一个 mode
- **THEN** `application` SHALL 继续使用统一的 mode 切换入口完成迁移
- **AND** SHALL NOT 直接写入 `session-runtime` 内部 `current_mode` 或等价 shadow state

#### Scenario: workflow orchestration 读取 runtime authoritative facts

- **WHEN** `application` 需要判断当前 session 的 mode、phase、active tasks 或 child 状态
- **THEN** 它 SHALL 通过 `session-runtime` 暴露的稳定快照或 query 接口读取
- **AND** SHALL NOT 重新从原始 runtime 内部字段拼装同类真相
