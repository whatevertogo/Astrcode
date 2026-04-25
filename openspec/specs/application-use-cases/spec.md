## Purpose

建立统一业务入口与治理边界的需求叙述基准，覆盖应用层对执行入口、权限与能力治理行为的稳定契约。
## Requirements

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

### Requirement: `application` SHALL 通过 app-owned session orchestration contracts 隔离 runtime 内部类型

`application` MUST 为编排场景定义 app-owned session orchestration contracts，并通过这些合同消费 `session-runtime` / `kernel` 提供的事实。用于 turn terminal、turn outcome、observe 摘要、recoverable parent delivery 等编排语义的 port 返回值 SHALL NOT 继续直接暴露 `session-runtime` 或 `kernel` 的内部快照类型。

#### Scenario: AgentSessionPort 不再暴露 runtime/kernel 内部快照
- **WHEN** `AgentSessionPort` 提供 observe、turn outcome、turn terminal 或 recoverable delivery 能力
- **THEN** 其返回类型 SHALL 使用 `application` 定义的 contract DTO
- **AND** SHALL NOT 继续直接暴露 `ProjectedTurnOutcome`、`TurnTerminalSnapshot`、`AgentObserveSnapshot`、`PendingParentDelivery` 或等价内部类型

#### Scenario: blanket impl 负责映射底层事实
- **WHEN** `SessionRuntime` 作为 `AppSessionPort` / `AgentSessionPort` 的实现被注入 `application`
- **THEN** blanket impl SHALL 在 port 层把 runtime/kernel 事实映射为 app-owned contracts
- **AND** `application` 用例本身 SHALL 不感知底层快照结构

#### Scenario: app-owned contracts 保持纯数据
- **WHEN** `application` 定义 session orchestration contracts
- **THEN** 这些 contracts SHALL 只包含纯数据字段与可序列化/可比较的业务结果
- **AND** SHALL NOT 直接承载 `CancelToken`、锁对象、原子状态、channel handle 或其他 runtime control primitive

### Requirement: `application` SHALL NOT 通过 `lib.rs` re-export 继续泄漏仅供编排内部使用的 runtime 类型

`application` crate 根导出面 MUST 只保留稳定业务入口、稳定业务摘要和确有必要的共享 surface。仅供内部编排使用的 runtime 类型 SHALL NOT 继续通过 `application::lib.rs` re-export 暴露给 `server` 或其他上层调用方。

#### Scenario: orchestration-only runtime types 从应用层根导出面移除
- **WHEN** 检查 `application::lib.rs`
- **THEN** 仅用于内部编排的 runtime 类型 SHALL 不再被 re-export
- **AND** 上层调用方 SHALL 通过 `App`、typed summary 或后续专门 surface 消费等价能力

#### Scenario: terminal authoritative facts 暂时保持稳定导出
- **WHEN** 某类 runtime facts 已经被 terminal / conversation surface 作为 authoritative read model 直接消费
- **THEN** `application` MAY 在本阶段继续保留必要导出
- **AND** 本次 change SHALL 聚焦编排合同隔离，不把 terminal read-model 全量迁移并入同一阶段

### Requirement: `application` SHALL 把 session 输入规范化留在 port 实现内部

`application` 用例层 MUST 把外部 session 输入视为原始请求数据；`session_id` 的规范化、typed conversion 与等价 runtime path helper 调用 SHALL 由 `AppSessionPort` / `AgentSessionPort` 的实现内部负责。应用层用例 SHALL NOT 直接调用 `astrcode_session_runtime::normalize_session_id` 或等价 helper。

#### Scenario: use case 只做字段校验，不做 runtime 规范化
- **WHEN** `application` 处理 session 相关请求
- **THEN** 它 MAY 做空值、格式非法等字段级校验
- **AND** SHALL NOT 直接依赖 runtime 的路径或 id 规范化 helper

#### Scenario: runtime 实现内部完成 session id 标准化
- **WHEN** 原始 `session_id` 进入 `AppSessionPort` / `AgentSessionPort` 的具体实现
- **THEN** 实现层 SHALL 在调用 runtime 内部逻辑前完成标准化与 typed conversion
- **AND** 该标准化语义 SHALL 与 `session-runtime` 内部 canonical helper 保持一致

---

### Requirement: `application` SHALL expose terminal session surface through app-owned contracts

`application` MUST 为 terminal / conversation surface 定义自己的稳定合同，并通过这些合同向 `server` 暴露 conversation snapshot、stream replay、rehydrate、control state、child summaries 与 slash candidates。`server` SHALL 只消费这些 application-owned contracts，SHALL NOT 继续直接依赖 runtime `Conversation*Facts`。

terminal / conversation 合同面至少 SHALL 覆盖：

- block
- delta
- patch
- status
- snapshot
- replay
- rehydrate
- authoritative summary 所需的 control / child / slash summaries

这些 contract 可以按模块拆分，但 `TerminalFacts.transcript` 与 `TerminalStreamReplayFacts.replay` 对外暴露的字段 MUST 属于 `application` 自己的类型，而不是 runtime snapshot / replay 类型别名。

#### Scenario: conversation snapshot 通过 application-owned facts 返回
- **WHEN** `server` 请求某个 session 的 conversation hydration snapshot
- **THEN** `application` SHALL 返回自身定义的 terminal / conversation snapshot contracts
- **AND** `server` SHALL NOT 直接处理 runtime `ConversationSnapshotFacts`

#### Scenario: terminal facts 不再直接承载 runtime transcript
- **WHEN** 检查 `application` 暴露给 `server` 的 `TerminalFacts`
- **THEN** `transcript` 字段 SHALL 是 application-owned snapshot contract
- **AND** SHALL NOT 直接使用 runtime `ConversationSnapshotFacts`

#### Scenario: conversation stream replay 通过 application-owned facts 返回
- **WHEN** `server` 请求某个 session 的 conversation stream replay 或 rehydrate 结果
- **THEN** `application` SHALL 返回自身定义的 replay / delta / rehydrate contracts
- **AND** `server` SHALL NOT 直接处理 runtime `ConversationStreamReplayFacts`

#### Scenario: terminal stream replay 不再直接承载 runtime replay
- **WHEN** 检查 `application` 暴露给 `server` 的 `TerminalStreamReplayFacts`
- **THEN** `replay` 字段 SHALL 是 application-owned replay contract
- **AND** SHALL NOT 直接使用 runtime `ConversationStreamReplayFacts`

#### Scenario: terminal surface contracts 保持纯数据
- **WHEN** 检查 `application` 暴露给 `server` 的 terminal / conversation surface 类型
- **THEN** 这些类型 SHALL 只包含纯数据字段
- **AND** SHALL NOT 直接承载 runtime projector、锁、channel handle 或其他运行时内部对象

---

### Requirement: `application` SHALL own stream projection coordination for terminal delta consumption

conversation stream 的 authoritative summary、catch-up replay 与 live delta projection MUST 由 `application` 拥有。`server` MAY 负责 SSE 订阅循环和 framing，但 SHALL NOT 直接实例化 runtime `ConversationStreamProjector` 或继续持有 runtime 专属 projection 状态。

#### Scenario: server 不再直接实例化 runtime stream projector
- **WHEN** `server` 处理 conversation SSE 路由
- **THEN** 它 SHALL 通过 `application` 暴露的 stream projection surface 获取 delta
- **AND** SHALL NOT 直接创建 runtime `ConversationStreamProjector`

#### Scenario: application 持有 projection 协调状态但不重写 runtime 算法
- **WHEN** `application` 为 conversation stream 暴露 projection coordination
- **THEN** 该协调状态 SHALL 归属于 `application`
- **AND** 内部 MAY 继续使用 runtime `ConversationStreamProjector`
- **AND** `server` SHALL 只消费 application 暴露的 replay / durable / live / recover surface

#### Scenario: authoritative summary 的合并逻辑留在 application
- **WHEN** 对话流需要根据 control state、child summaries 与 slash candidates 生成附加 delta
- **THEN** 这些 authoritative summary 的比较与合并 SHALL 由 `application` 负责
- **AND** `server` SHALL 只负责把结果映射成 protocol DTO

---

### Requirement: `application` SHALL own session creation validation at the server boundary

`server -> application` 边界上的 session create 输入校验 MUST 由 `application` use case 拥有。`server` MAY 做空值与 JSON 形状校验，但 SHALL NOT 直接调用 runtime `normalize_working_dir` 或等价路径 helper。

#### Scenario: create session route 不直接调用 runtime working-dir helper
- **WHEN** `server` 处理创建 session 的 HTTP 请求
- **THEN** 工作目录规范化与合法性校验 SHALL 由 `application` use case 或其 port 实现处理
- **AND** route 层 SHALL NOT 直接调用 runtime 路径 helper

#### Scenario: 非法 working directory 通过 application error 返回
- **WHEN** 用户提交不存在、非法或不是目录的 `workingDir`
- **THEN** `application` SHALL 返回明确的业务错误
- **AND** `server` 只负责把该错误映射成 HTTP 响应

---

### Requirement: `application` SHALL hide runtime fork result behind app-owned fork surface

`server -> application` 的 fork 输入 MUST 使用 application-owned selector，而 runtime `ForkPoint` 与 `ForkResult` SHALL 留在 application port / session-runtime 内部。`App::fork_session()` 对 `server` 的稳定返回值 SHALL 是 `SessionMeta`。

#### Scenario: App::fork_session 不向 server 暴露 runtime ForkResult
- **WHEN** `server` 调用 `App::fork_session`
- **THEN** 它 SHALL 收到 `SessionMeta`
- **AND** SHALL NOT 观察 runtime `ForkResult` 的字段结构

---

### Requirement: `application` 通过治理端口消费运行时协调，而不拥有设施 owner

`application` SHALL 通过治理端口消费进程级运行时协调、治理快照与关闭能力；这些设施 owner 不再由 `core` 持有，也不要求 `application` 自己成为设施 owner。

#### Scenario: application governance does not require core-owned runtime coordinator

- **WHEN** `application` 需要读取治理快照、协调关闭或消费运行时状态
- **THEN** 它 SHALL 通过稳定治理端口完成
- **AND** 不要求直接持有 `RuntimeCoordinator` 这类组合根设施 owner

#### Scenario: application depends on contracts rather than core-owned mutable state

- **WHEN** `application` 需要协调会话运行时、治理快照或关闭行为
- **THEN** 它 SHALL 通过稳定 port 与值对象完成编排
- **AND** 不依赖 `core` 中的全局可变状态 owner

---

### Requirement: `application` 编排项目路径与环境副作用契约，而不直接持有实现

凡是与 project dir、working dir 归一化、tool result durable persist 等环境副作用相关的业务编排，`application` SHALL 依赖稳定契约完成；具体实现 SHALL 留在 adapter 或 `astrcode-support` 这类受限 support crate。

#### Scenario: application does not use core filesystem helpers directly

- **WHEN** 某个应用层用例需要校验 project dir、归一化 working dir 或触发 durable persist
- **THEN** `application` SHALL 通过稳定 port 编排这些能力
- **AND** 不直接调用 `core` 中的具体文件系统 helper
- **AND** 若需要共享宿主路径解析，SHALL 通过 `astrcode-support::hostpaths` 或等价稳定契约消费

#### Scenario: application does not resolve home directories from core

- **WHEN** 应用层需要定位 Astrcode home、project root 或等价宿主路径
- **THEN** 它 SHALL 通过组合根注入的能力、`astrcode-support::hostpaths` 或 adapter 契约完成
- **AND** 不把 `core` 作为 home 目录解析 owner
