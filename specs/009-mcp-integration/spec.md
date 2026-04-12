# Feature Specification: MCP Server 接入支持

**Feature Branch**: `009-mcp-integration`
**Created**: 2026-04-12
**Status**: Draft
**Input**: User description: "为项目增加接入mcp支持"

## Clarifications

### Session 2026-04-12

- Q: 远程 MCP 传输模式 → A: 支持 stdio + Streamable HTTP，SSE 作为兼容回退（参考 Claude Code 实现，平衡协议前瞻性和生态覆盖面）
- Q: stdio 安全边界 → A: 在 Assumptions 中明确信任模型——配置文件的修改者被完全信任
- Q: 工具名冲突策略 → A: 先注册者优先，但冲突时必须有可观测的日志或 UI 提示，不能静默跳过
- Q: 远程服务器认证 → A: 支持 HTTP headers（静态 + 环境变量展开）和 OAuth 流程
- Q: SC-001 严谨性 → A: 拆分为"系统侧处理延迟"和"端到端延迟（受服务器影响）"
- Q: 取消信号 → A: 发送取消通知后，若服务器未在合理时间内响应，客户端应强制断开连接
- Q: 热加载与正在进行的工具调用 → A: 等待当前调用完成后再断开，设置超时上限
- Q: 输出大小限制 → A: 复用已有的 TOOL_RESULT_INLINE_LIMIT 落盘机制，不单独实现 MCP 专用限制

## User Scenarios & Testing *(mandatory)*

### User Story 1 - 连接外部 MCP 服务器并使用其工具 (Priority: P1)

用户在配置文件中声明一个或多个 MCP 服务器（指定启动命令或远程地址），系统启动后自动连接这些服务器，将其提供的工具注册到能力路由中。Agent 在对话过程中可以像调用内置工具一样调用 MCP 服务器提供的工具。

**Why this priority**: 这是 MCP 接入的核心价值——扩展 Agent 可用工具集。没有这个能力，后续所有场景都无从谈起。

**Independent Test**: 可以通过在配置文件中声明一个简单的 MCP 服务器（如 echo 测试服务器），启动应用后验证 Agent 能成功发现并调用该工具来完整测试。

**Acceptance Scenarios**:

1. **Given** 用户已在配置文件中声明了一个 MCP 服务器（stdio 模式），**When** 系统启动并完成初始化，**Then** 该服务器的工具出现在可用工具列表中，Agent 可以在对话中调用这些工具并获取正确结果
2. **Given** 用户已在配置文件中声明了一个 MCP 服务器（Streamable HTTP 远程模式），**When** 系统启动并完成初始化，**Then** 该服务器的工具出现在可用工具列表中，Agent 可以在对话中调用这些工具并获取正确结果
3. **Given** 用户已在配置文件中声明了一个 MCP 服务器（SSE 远程模式），**When** 系统尝试连接，**Then** 系统通过 SSE 兼容回退方式连接该服务器并注册其工具
4. **Given** 系统已连接多个 MCP 服务器，**When** 不同服务器声明了同名工具，**Then** 系统按先注册者优先策略处理冲突，在日志中记录被跳过的工具名和冲突来源，不阻塞其他服务器的加载

---

### User Story 2 - 管理 MCP 服务器连接生命周期 (Priority: P2)

系统在运行过程中持续监控 MCP 服务器的连接状态。当服务器进程崩溃或网络连接断开时，系统自动尝试重连；当用户修改配置时，系统能热加载新增的服务器并断开已移除的服务器，无需重启整个应用。

**Why this priority**: 生产环境下 MCP 服务器可能不稳定，自动重连和热加载能显著提升用户体验。但这依赖于 P1 的基础连接能力。

**Independent Test**: 可以通过启动后手动杀掉 MCP 服务器进程来验证重连机制，或通过修改配置文件验证热加载。

**Acceptance Scenarios**:

1. **Given** 一个已连接的 stdio MCP 服务器进程意外退出，**When** 系统检测到连接断开，**Then** 系统自动尝试重新启动该服务器进程并恢复连接（指数退避，最多 5 次）
2. **Given** 一个已连接的远程 MCP 服务器网络中断，**When** 系统检测到连接断开，**Then** 系统自动尝试重连（指数退避，初始 1 秒，最大 30 秒，最多 5 次）
3. **Given** 系统正在运行，**When** 用户在配置文件中新增一个 MCP 服务器声明，**Then** 系统在文件监听触发后自动连接新服务器并注册其工具
4. **Given** 系统正在运行，**When** 用户从配置文件中移除一个 MCP 服务器声明，**Then** 系统等待该服务器正在进行的工具调用完成（或超时后强制断开），然后断开连接并注销其工具
5. **Given** 一个 MCP 服务器多次重连失败，**When** 失败次数超过阈值（5 次），**Then** 系统将该服务器标记为不可用，不再继续重试，并在界面上提示用户

---

### User Story 3 - 通过 MCP 接收服务器推送的 Prompt 指令 (Priority: P3)

MCP 服务器可以在握手时返回 instructions，系统在 prompt 组装阶段将这些指令注入到 Agent 的上下文中。MCP 服务器还可以通过 `prompts/list` 暴露 prompt 模板，作为可调用的命令注册到系统中。

**Why this priority**: 这让 MCP 服务器能主动提供使用指导，提升 Agent 调用工具的准确率。但它是增强特性，基础工具调用（P1）已能满足核心需求。

**Independent Test**: 可以通过声明一个提供 instructions 的 MCP 服务器，验证 Agent 发送请求时上下文中包含该 instruction 内容。

**Acceptance Scenarios**:

1. **Given** 一个 MCP 服务器在握手响应中返回了 instructions 字段，**When** 系统完成与该服务器的握手，**Then** 这些 instructions 被收集并注入到 prompt 组装管线中
2. **Given** 一个 MCP 服务器支持 `prompts/list` 能力，**When** 系统完成与该服务器的握手，**Then** 服务器的 prompt 模板被注册为可调用的命令
3. **Given** 多个 MCP 服务器声明了相同 block_id 的 prompt，**When** 系统组装 prompt，**Then** 按优先级规则去重，不会出现重复注入

---

### User Story 4 - 通过 MCP 使用服务器提供的资源 (Priority: P4)

MCP 服务器可以通过 `resources/list` 和 `resources/read` 暴露结构化数据（如文件、数据库记录、API 响应）。系统提供两个内置工具（ListMcpResources 和 ReadMcpResource）让 Agent 可以在对话中查询和引用这些资源。

**Why this priority**: 资源能力是 MCP 协议的重要组成部分，但大多数工具型 MCP 服务器不依赖此功能。优先级低于 prompt 注入。

**Independent Test**: 可以通过声明一个暴露资源列表的 MCP 服务器，验证 Agent 能列出并读取这些资源。

**Acceptance Scenarios**:

1. **Given** 一个 MCP 服务器通过 `resources/list` 暴露了资源列表，**When** Agent 调用 ListMcpResources 工具，**Then** 返回该服务器所有可用资源的 URI、名称和描述
2. **Given** 一个 MCP 服务器暴露了资源，**When** Agent 调用 ReadMcpResource 工具并指定 URI，**Then** 返回该资源的内容；二进制内容被持久化到磁盘并返回文件路径

---

### User Story 5 - 项目级 MCP 服务器审批 (Priority: P5)

当项目配置文件（如 `.mcp.json`）中声明了 MCP 服务器时，系统在首次连接前向用户展示审批对话框。用户可以选择批准当前服务器、批准所有项目服务器、或拒绝。审批决策保存在本地设置中，不会被提交到版本控制。

**Why this priority**: 项目级配置可能来自多人协作或开源项目，恶意 command 可能执行任意代码。审批机制是安全基础，但它的使用频率低于核心连接功能。

**Independent Test**: 可以通过在项目中创建 `.mcp.json` 配置文件，验证首次启动时弹出审批对话框。

**Acceptance Scenarios**:

1. **Given** 项目配置文件中声明了新的 MCP 服务器，**When** 系统首次发现该服务器，**Then** 弹出审批对话框，显示服务器名称和配置摘要
2. **Given** 用户选择了"批准此项目所有 MCP 服务器"，**When** 后续项目配置中新增服务器，**Then** 自动批准连接，不再弹窗
3. **Given** 用户拒绝了某个服务器，**When** 系统下次启动，**Then** 该服务器被标记为 disabled，不尝试连接

---

### Edge Cases

- MCP 服务器启动命令不存在或执行失败时 → 记录错误日志，将该服务器标记为 failed，不阻塞其他服务器
- MCP 服务器返回的工具 schema 不符合 JSON Schema 规范时 → 拒绝注册该工具，记录警告日志
- MCP 服务器在工具调用过程中超时 → 发送 `notifications/cancelled`，若服务器未在合理时间内响应则强制断开连接，向 Agent 返回超时错误
- 配置文件中声明的 MCP 服务器数量很多时 → 本地服务器并行初始化（并发度 ≤ 3），远程服务器并行初始化（并发度 ≤ 10）
- MCP 协议版本不匹配时 → 在握手阶段检测版本兼容性，不兼容时拒绝连接并提示用户
- 热加载移除服务器时有正在进行的工具调用 → 等待调用完成（超时上限，如 30 秒），超时后强制断开
- MCP 服务器支持 `tools/list_changed` 通知 → 清除工具缓存并重新获取工具列表
- 配置中引用了不存在的环境变量（如 `${MISSING_KEY}`）→ 记录警告日志，将空值传给服务器
- 同一个 MCP 服务器在多个配置作用域中声明 → 基于签名去重（stdio 按 command+args，远程按 URL），高优先级配置覆盖低优先级
- MCP 工具返回超大结果 → 复用已有落盘机制（TOOL_RESULT_INLINE_LIMIT），超过阈值时持久化到磁盘并返回读取指令

## Requirements *(mandatory)*

### Functional Requirements

**连接与传输**

- **FR-001**: 系统 MUST 支持通过配置文件声明 MCP 服务器，支持三种传输模式：stdio（本地进程）、Streamable HTTP（推荐远程模式）、SSE（兼容回退远程模式）
- **FR-002**: 系统 MUST 在启动时自动发现并连接所有已声明的 MCP 服务器，完成 MCP 协议握手（`initialize` + 能力协商）
- **FR-003**: 系统 MUST 根据传输类型采用不同的并发策略连接服务器：本地服务器并发度 ≤ 3，远程服务器并发度 ≤ 10

**工具桥接**

- **FR-004**: 系统 MUST 将 MCP 服务器通过 `tools/list` 声明的工具转换为符合 `CapabilityInvoker` 接口的能力，并注册到 `CapabilityRouter` 中
- **FR-005**: 系统 MUST 使用 `mcp__{serverName}__{toolName}` 命名规范注册 MCP 工具，确保与内置工具和其他插件工具不冲突
- **FR-006**: 系统 MUST 将 Agent 的工具调用请求转换为 MCP 协议的 `tools/call` 请求发送到对应服务器，并将响应结果转换回 `ToolExecutionResult`
- **FR-007**: 系统 MUST 从 MCP 工具的 `annotations` 中提取能力提示（readOnlyHint、destructiveHint、openWorldHint），用于策略引擎的权限判断

**连接生命周期**

- **FR-008**: 系统 MUST 在 MCP 服务器连接断开时自动尝试重连，重连策略采用指数退避（初始 1 秒，每次翻倍，最大 30 秒），最多重试 5 次
- **FR-009**: 系统 MUST 仅对远程传输（Streamable HTTP、SSE）支持自动重连；stdio 服务器断开后不自动重启进程
- **FR-010**: 系统 MUST 支持在运行时动态增减 MCP 服务器（热加载），无需重启应用

**安全与权限**

- **FR-011**: 系统 MUST 将 MCP 服务器提供的工具纳入策略引擎的权限管理，与内置工具和插件工具统一管控
- **FR-012**: 系统 MUST 将 MCP 来源的工具在 Skill 覆盖优先级链中正确排序（`builtin < mcp < plugin < user < project`）
- **FR-013**: 系统 MUST 在 MCP 工具调用过程中支持取消操作：发送 `notifications/cancelled`，若服务器在合理时间内未响应则强制断开连接
- **FR-014**: 系统 MUST 对项目配置文件中声明的 MCP 服务器执行首次连接审批流程，审批决策存储在本地设置中
- **FR-015**: 系统 MUST 支持通过策略允许列表和拒绝列表控制哪些 MCP 服务器可以被连接（按名称、命令或 URL 匹配）

**远程认证**

- **FR-016**: 系统 MUST 支持在远程 MCP 服务器配置中声明静态 HTTP headers（用于 API Key 等认证方式）
- **FR-017**: 系统 MUST 支持配置值中的环境变量展开（如 `${API_TOKEN}`），缺失的环境变量记录为警告
- **FR-018**: 系统 SHOULD 支持远程 MCP 服务器的 OAuth 认证流程，包括动态客户端注册（DCR）、浏览器授权和 token 刷新

**Prompt 与资源**

- **FR-019**: 系统 MUST 将 MCP 服务器握手响应中的 `instructions` 字段注入到 prompt 组装管线中
- **FR-020**: 系统 SHOULD 支持通过 MCP 的 `prompts/list` 和 `prompts/get` 能力将服务器 prompt 模板注册为可调用命令
- **FR-021**: 系统 SHOULD 支持通过内置工具（ListMcpResources、ReadMcpResource）暴露 MCP 资源数据给 Agent

**可靠性与隔离**

- **FR-022**: 系统 MUST 在 MCP 服务器初始化失败时不阻塞其他服务器或内置工具的加载
- **FR-023**: 系统 MUST 复用已有的 `TOOL_RESULT_INLINE_LIMIT` 落盘机制处理 MCP 工具的调用结果，超过阈值的结果持久化到磁盘而非截断
- **FR-024**: 系统 MUST 在 MCP 工具调用超时时返回明确的错误信息，不永久挂起 Agent 循环
- **FR-025**: 系统 MUST 在工具名冲突时记录明确的警告日志（包含冲突的工具名和两个来源的标识），不能静默跳过
- **FR-026**: 系统 MUST 支持监听 MCP 服务器推送的 `tools/list_changed`、`prompts/list_changed`、`resources/list_changed` 通知，并相应更新已注册的能力

**配置管理**

- **FR-027**: 系统 MUST 支持多个配置作用域：user（全局）、project（项目级 `.mcp.json`）、local（项目本地私有设置），按优先级从低到高合并
- **FR-028**: 系统 MUST 基于配置签名进行去重：stdio 按 command+args 签名，远程按 URL 签名，高优先级配置覆盖低优先级

### Key Entities

- **MCP Server Config**: 用户对 MCP 服务器的声明配置，包括名称、传输模式（stdio/streamable-http/sse）、启动命令或 URL、参数、环境变量、HTTP headers、超时设置、配置作用域
- **MCP Connection**: 与 MCP 服务器建立的活动连接，包含传输通道、握手协商结果（capabilities、serverInfo、instructions）、连接状态（connected/failed/needs-auth/pending/disabled）
- **MCP Tool Bridge**: 将 MCP 服务器声明的单个工具桥接为 `CapabilityInvoker` 的适配层，负责请求/响应的协议转换、annotations 提取、结果大小限制
- **MCP Server Registry**: 所有已声明的 MCP 服务器的注册表，追踪配置状态、连接状态和健康状态，支持基于签名的去重
- **MCP Approval State**: 项目级 MCP 服务器的审批状态（approved/rejected/pending），存储在本地设置中，不随项目版本控制

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 系统侧处理延迟（从配置解析完成到发起连接）不超过 500ms；端到端连接时间主要取决于 MCP 服务器本身的启动/响应速度
- **SC-002**: 系统同时连接 10 个 MCP 服务器时，Agent 调用任一工具的响应延迟不超过单次直连的 1.2 倍
- **SC-003**: 当一个远程 MCP 服务器断开后，系统能在指数退避重试周期内（约 30 秒）自动恢复连接并重新注册工具
- **SC-004**: 单个 MCP 服务器初始化失败不影响其他 MCP 服务器和内置工具的正常使用（100% 隔离）
- **SC-005**: 修改 MCP 配置后，系统能在 10 秒内完成热加载（新增连接或等待已有调用完成后移除）
- **SC-006**: 工具名冲突时，用户能通过日志或状态面板发现被跳过的工具名及其冲突来源

## Assumptions

- MCP 服务器遵循 MCP 规范（2025-03-26 或更高版本），支持 `initialize`、`tools/list`、`tools/call` 等基础能力
- stdio 模式的 MCP 服务器是可执行的二进制或脚本，通过命令行启动并使用 stdin/stdout 进行 JSON-RPC 通信
- Streamable HTTP 是 MCP 协议推荐的远程传输方式，SSE 作为旧版兼容保留
- 配置格式沿用项目现有的 JSON 配置体系（`~/.astrcode/config.json`），项目级 MCP 配置使用独立 `.mcp.json`（与 Claude Code 生态兼容）
- **安全信任模型**: 配置文件的修改者被完全信任。stdio 模式的 command 字段本质上是任意命令执行，v1 不对命令内容做安全审查
- MCP 服务器的工具名称可能与内置工具或其他插件工具冲突，采用先注册者优先的策略，但冲突必须被记录
- MCP 集成不依赖现有的 plugin crate（两者是并行的外部能力来源），但共享 `CapabilityRouter` 注册机制
- v1 版本不需要支持 MCP sampling（让 MCP 服务器反向调用 LLM）
- MCP 工具的输出大小限制复用已有的 `TOOL_RESULT_INLINE_LIMIT` 机制，不单独实现 MCP 专用限制
- stdio 服务器断开后不自动重启进程（进程崩溃视为需要用户干预的事件）
