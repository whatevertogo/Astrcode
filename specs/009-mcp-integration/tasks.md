# Tasks: MCP Server 接入支持

**Input**: Design documents from `/specs/009-mcp-integration/`
**Prerequisites**: plan.md (required), spec.md (required for user stories), research.md, data-model.md, contracts/

**Tests**: 本项目要求 `cargo test` 验证所有改动。MCP 涉及外部进程通信和协议交互，需要 mock 传输层单元测试 + 真实 MCP 服务器集成测试。

**Organization**: Tasks 按 user story 组织，每个 story 可独立实现和测试。

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

## Path Conventions

- **Crate 根目录**: `crates/runtime-mcp/src/`
- **Manager 目录**: `crates/runtime-mcp/src/manager/`（connection、reconnect、hot_reload 内聚于此）
- **Config 目录**: `crates/runtime-mcp/src/config/`（settings_port.rs 定义纯接口 trait）
- **Transport 目录**: `crates/runtime-mcp/src/transport/`（mock.rs 为 `#[cfg(test)]` 条件编译）
- **修改现有文件**: `crates/runtime/src/`, `crates/server/src/routes/mcp.rs`
- **Cargo.toml**: `crates/runtime-mcp/Cargo.toml`, `Cargo.toml` (workspace)

## Constitution-Driven Task Rules

- runtime-mcp 仅依赖 core，不依赖 runtime（宪法：runtime-...-loader 系列 MUST 依赖 core 而非 runtime）
- runtime surface assembler 扩展不创建第二套注册路径（宪法 II: One Boundary, One Owner）
- 所有异步操作有取消机制，不在持锁状态下 await（宪法 VI: Runtime Robustness）
- 关键操作有结构化日志，错误不静默忽略（宪法 VII: Observability）
- 单个文件不超过 800 行（宪法：runtime 门面约束）
- 审批状态等 settings 的读写接口定义在 core（纯契约），runtime-mcp 仅依赖 core 的接口，不依赖 runtime 或 runtime-config

## 关键设计决策

- **runtime 集成保护**: T026/T027 修改 runtime_surface_assembler 和 bootstrap 时，使用 `Option<McpConnectionManager>` 包装。MCP 初始化失败时 runtime 照常启动，只记录警告日志。不使用 feature flag——MCP 代码始终编译，只是运行时可能无配置而不激活。
- **in-flight 追踪**: T020 创建 McpConnectionManager 时建立 `Arc<AtomicUsize>` 计数器追踪进行中的工具调用，T036 基于此实现优雅断开。
- **热加载文件监听**: 使用 workspace 已有的 `notify` crate，回调通过 `tokio::sync::mpsc` 通道安全触发异步 reload。
- **settings 接口边界**: `McpSettingsStore` trait 定义在 `config/settings_port.rs`（纯接口层），`McpApprovalManager` 通过 trait 读写审批数据，具体实现由 runtime 在 bootstrap 时注入——runtime-mcp 不直接读写 settings 文件，不依赖 runtime 或 runtime-config

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: 创建 crate 骨架、协议类型定义、项目配置

- [ ] T001 创建 `crates/runtime-mcp/Cargo.toml`，依赖 astrcode-core、astrcode-protocol、tokio、reqwest、serde、serde_json、async-trait、thiserror、log、futures-util、notify；在 workspace `Cargo.toml` 中注册新 member
- [ ] T002 [P] 创建 `crates/runtime-mcp/src/lib.rs`，仅声明 Phase 1/2 已有的子模块（protocol、transport、config、manager），导出公共类型；后续 Phase 开始时再声明新模块（bridge 在 Phase 3、hot_reload 在 manager 目录内无需额外声明）
- [ ] T003 [P] 实现 `crates/runtime-mcp/src/protocol/mod.rs`：定义 JSON-RPC 2.0 消息类型（JsonRpcRequest、JsonRpcResponse、JsonRpcNotification、JsonRpcError）和序列化/反序列化
- [ ] T004 [P] 实现 `crates/runtime-mcp/src/protocol/types.rs`：MCP DTO 类型（McpToolInfo、McpPromptInfo、McpResourceInfo、McpToolAnnotations、McpServerInfo、McpServerCapabilities、InitializeParams、InitializeResult 等）
- [ ] T005 [P] 实现 `crates/runtime-mcp/src/protocol/error.rs`：MCP 协议错误类型（McpProtocolError、McpTransportError、McpTimeoutError），实现 From 转换到 AstrError
- [ ] T006 [P] 实现 `crates/runtime-mcp/src/config/types.rs`：配置数据类型（McpServerConfig、McpTransportConfig、McpConfigScope、McpOAuthConfig），从 JSON 反序列化
- [ ] T007 确认 `cargo build -p astrcode-runtime-mcp` 通过

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: 传输层抽象和 MCP 协议客户端——所有 user story 的基础

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [ ] T008 实现 `crates/runtime-mcp/src/transport/mod.rs`：定义 `McpTransport` trait（start、send_request、send_notification、close、is_active、transport_type）
- [ ] T009 实现 `crates/runtime-mcp/src/transport/stdio.rs`：`StdioTransport`，使用 `tokio::process::Command` 启动子进程，stdin/stdout 做 JSON-RPC 传输，按 SIGINT → SIGTERM → SIGKILL 顺序优雅关闭
- [ ] T010 实现 `crates/runtime-mcp/src/protocol/client.rs`：`McpClient`——封装 MCP 握手（initialize + initialized 通知）、协议版本兼容性检查、server_info/capabilities/instructions 存储
- [ ] T011 为 McpClient 实现 `list_tools()` 和 `call_tool()` 方法：构造 JSON-RPC 请求、解析响应、处理错误码
- [ ] T012 为 McpClient 实现 `send_cancel()` 方法：发送 `notifications/cancelled` 通知，并返回 cancel request_id 供调用方设置超时断开计时器
- [ ] T013 为 McpClient 实现 `on_list_changed()` 方法：注册 list_changed 通知的异步回调处理器
- [ ] T014 [P] 实现 `crates/runtime-mcp/src/manager/connection.rs`：`McpConnection` 状态机（Pending/Connecting/Connected/Failed/NeedsAuth/Disabled）及状态转换逻辑；创建 `manager/` 目录，`manager/mod.rs` 仅声明 connection 子模块（Phase 3 会扩展）
- [ ] T015 创建 `crates/runtime-mcp/src/transport/mock.rs`（测试用，`#[cfg(test)]` 条件编译）：mock 传输实现，支持预设响应序列、请求验证、可编程断连行为（用于测试重连）
- [ ] T016 为 McpClient 握手、list_tools、call_tool 编写单元测试（使用 mock 传输），验证协议流程正确性——在 `crates/runtime-mcp/src/protocol/client.rs` 底部 `#[cfg(test)]` 模块中
- [ ] T016b 为远程传输异常场景编写 mock 测试：SSE 断流后重连、HTTP 5xx 错误处理、请求超时触发 cancel 流程——在 `transport/mock.rs` 底部 `#[cfg(test)]` 模块中，验证 McpClient 在传输层异常时的行为
- [ ] T017 确认 `cargo test -p astrcode-runtime-mcp` 通过

**Checkpoint**: 传输层抽象和 MCP 协议客户端就绪，可开始 user story 实现

---

## Phase 3: User Story 1 - 连接外部 MCP 服务器并使用其工具 (Priority: P1) 🎯 MVP

**Goal**: 用户声明 MCP 服务器后，系统自动连接并注册工具，Agent 可调用

**Independent Test**: 在 `.mcp.json` 中声明一个 MCP 服务器，启动后验证工具出现在可用列表中，Agent 可调用并获取结果

### Implementation for User Story 1

- [ ] T018 [P] [US1] 实现 `crates/runtime-mcp/src/bridge/tool_bridge.rs`：`McpToolBridge` struct 实现 `Tool` trait，桥接 MCP 工具到 Astrcode Tool 接口（definition 从 MCP inputSchema 映射、execute 调用 McpClient.call_tool、annotations 映射到 capability metadata）；工具执行结果复用 `ToolContext.resolved_inline_limit` 决定是否落盘
- [ ] T019 [P] [US1] 实现 `crates/runtime-mcp/src/bridge/mod.rs`：bridge 模块公共导出和辅助函数（build_mcp_tool_name 生成 `mcp__{server}__{tool}` 格式名称）；在 lib.rs 中声明 bridge 模块
- [ ] T020 [US1] 实现 `crates/runtime-mcp/src/manager/mod.rs`：`McpConnectionManager`——批量连接所有已声明服务器（本地并发度 ≤ 3，远程并发度 ≤ 10），单个失败不阻塞其他，返回 `McpConnectionResults`；内部维护 `Arc<AtomicUsize>` 进行中调用计数器
- [ ] T021 [US1] 为 McpConnectionManager 实现 `connect_one()` 方法：连接单个服务器、获取工具列表、创建 McpToolBridge、通过 ToolCapabilityInvoker 包装为 CapabilityInvoker；连接成功后注册 list_changed handler（收到通知时清除工具缓存并重新 list_tools）
- [ ] T022 [US1] 为 McpConnectionManager 实现 `ManagedRuntimeComponent` trait（component_name、shutdown_component）
- [ ] T023 [US1] 实现 `crates/runtime-mcp/src/config/loader.rs`：`McpConfigManager`——从 `.mcp.json` 和 settings 加载配置、环境变量展开（`${VAR}` 缺失变量返回 Err 并明确指出缺失的变量名）、签名去重（stdio 按 command:args，远程按 URL）
- [ ] T024 [US1] 实现 `crates/runtime-mcp/src/config/settings_port.rs`：定义 `McpSettingsStore` trait（load/save 审批数据）和 `McpApprovalData` DTO——纯接口层，无 IO 实现；实现 `crates/runtime-mcp/src/config/approval.rs`：`McpApprovalManager`——项目级服务器审批状态管理（approved/rejected/pending），通过 `McpSettingsStore` trait 读写审批数据，具体实现由 runtime 在 bootstrap 时注入
- [ ] T025 [US1] 实现 `crates/runtime-mcp/src/config/policy.rs`：`McpPolicyFilter`——策略允许/拒绝列表过滤（按名称、命令、URL 匹配，拒绝优先于允许）
- [ ] T026 [US1] 修改 `crates/runtime/src/runtime_surface_assembler.rs`：新增 MCP 初始化路径——将 `Option<Arc<McpConnectionManager>>` 作为参数传入 `assemble_runtime_surface`；MCP 为 None 或初始化失败时跳过 MCP 贡献，不影响内置工具和插件注册；将 MCP 工具注入到 AssembledRuntimeSurface（复用已有 `RuntimeSurfaceContribution` 结构）
- [ ] T027 [US1] 修改 `crates/runtime/src/bootstrap.rs`：在 bootstrap 流程中尝试加载 MCP 配置（`Option<McpConfigManager>`），成功则创建 McpConnectionManager 并传入 assembler；失败则记录警告日志并传入 None；确保 MCP 不可用时 runtime 照常启动
- [ ] T028 [US1] 实现 `crates/runtime-mcp/src/transport/http.rs`：`StreamableHttpTransport`——使用 reqwest 发送 HTTP POST，接收 SSE 响应流，支持静态 headers 注入（从 McpTransportConfig 读取）
- [ ] T029 [US1] 实现 `crates/runtime-mcp/src/transport/sse.rs`：`SseTransport`（兼容回退）——SSE 连接 + HTTP POST 请求，支持静态 headers 注入
- [ ] T030 [US1] 为 McpToolBridge 编写单元测试（验证 Tool trait 实现、名称生成、annotations 映射），在 `bridge/tool_bridge.rs` 底部 `#[cfg(test)]` 模块
- [ ] T031 [US1] 为 McpConfigManager 编写单元测试（配置加载、去重、环境变量缺失返回 Err），在 `config/loader.rs` 底部 `#[cfg(test)]` 模块
- [ ] T032 [US1] 为 McpConnectionManager 编写单元测试（批量连接、错误隔离、工具注册、工具名冲突时记录明确警告日志含两个来源标识——验证 FR-025），使用 mock 传输，在 `manager/mod.rs` 底部 `#[cfg(test)]` 模块
- [ ] T033 [US1] 确认 `cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test -p astrcode-runtime-mcp` 通过

**Checkpoint**: MCP 服务器可连接、工具可注册、Agent 可调用——MVP 完成

---

## Phase 4: User Story 2 - 管理 MCP 服务器连接生命周期 (Priority: P2)

**Goal**: 服务器断开时自动重连，配置变更时热加载

**Independent Test**: 启动后杀掉 MCP 服务器进程验证重连，或修改配置文件验证热加载

### Implementation for User Story 2

- [ ] T034 [US2] 实现 `crates/runtime-mcp/src/manager/reconnect.rs`：自动重连逻辑——仅远程传输（HTTP/SSE），指数退避（1s → 2s → 4s → 8s → 16s, max 30s），最多 5 次，使用 tokio::spawn 持有 JoinHandle；重连前检查 `is_disabled()` 标志；stdio 传输不重连（直接返回）
- [ ] T035 [US2] 实现连接断开检测：监听 transport 的 is_active 状态变化和 McpClient 内部错误回调，调用 `reconnect.rs` 中的重连流程；区分 stdio（不重连）和远程传输（触发重连）
- [ ] T036a [US2] 在 `manager/mod.rs` 中实现 in-flight 请求追踪：`AtomicUsize` 计数器在工具调用开始时 +1、完成（含错误）时 -1；提供 `in_flight_count()` 和 `wait_idle(timeout)` 方法
- [ ] T036 [US2] 为 McpConnectionManager 实现 `disconnect_one()` 方法：先调用 `wait_idle(30s)` 等待进行中调用完成，超时后强制断开 transport，然后清除缓存（tools/commands/resources 的 memoize cache）
- [ ] T036c [US2] 实现 cancel + 强制断开逻辑：工具调用 cancel 时发送 `notifications/cancelled`，设置 30 秒计时器，超时后调用 transport.close() 强制断开连接并向 Agent 返回超时错误——验证 FR-013
- [ ] T037 [US2] 实现 `crates/runtime-mcp/src/manager/hot_reload.rs`：使用 `notify` crate 监听 `.mcp.json` 和 settings 文件变更；通过 `tokio::sync::mpsc` 通道发送变更事件，避免文件监听线程直接触碰异步状态
- [ ] T038 [US2] 为 McpConnectionManager 实现 `reload_config()` 方法：接收 mpsc 变更事件，调用 McpConfigManager.load_all() 获取新配置，对比差异后新增调用 connect_one()、移除调用 disconnect_one()、未变化保持不变
- [ ] T039 [US2] 为 McpConnectionManager 实现 `connected_invokers()` 方法：返回当前所有已连接服务器的 CapabilityInvoker 列表，供 runtime 重新注册到 CapabilityRouter
- [ ] T040 [US2] 修改 runtime bootstrap 集成热加载：启动 notify 文件监听，变更事件通过 mpsc 传递到 runtime 的 tokio task，触发 reload_config 并用 connected_invokers() 更新 CapabilityRouter
- [ ] T041 [US2] 为重连策略编写单元测试（指数退避、最大次数、取消机制、stdio 不重连），在 `manager/reconnect.rs` 底部 `#[cfg(test)]` 模块
- [ ] T042 [US2] 为热加载编写单元测试（新增/移除/变更检测、mpsc 通道正确传递事件），在 `manager/hot_reload.rs` 底部 `#[cfg(test)]` 模块
- [ ] T043 [US2] 确认 `cargo test -p astrcode-runtime-mcp` 通过

**Checkpoint**: MCP 服务器可自动重连、配置可热加载

---

## Phase 5: User Story 3 - 通过 MCP 接收服务器推送的 Prompt 指令 (Priority: P3)

**Goal**: MCP 服务器的 instructions 注入到 prompt 组装管线，prompt 模板注册为可调用命令

**Independent Test**: 连接一个提供 instructions 的 MCP 服务器，验证 prompt 组装结果中包含该指令

### Implementation for User Story 3

- [ ] T044 [P] [US3] 实现 `crates/runtime-mcp/src/bridge/prompt_bridge.rs`：将 MCP 服务器握手响应中的 `instructions` 转换为 `PromptDeclaration`（source 设为 Mcp，origin 设为服务器名）
- [ ] T045 [P] [US3] 为 McpClient 实现 `list_prompts()` 和 `get_prompt()` 方法：调用 MCP `prompts/list` 和 `prompts/get`，解析返回的 prompt 模板
- [ ] T046 [US3] 修改 McpConnectionManager：在连接成功后收集 instructions 和 prompt 模板，加入 `McpConnectionResults.prompt_declarations`
- [ ] T047 [US3] 修改 runtime_surface_assembler：将 MCP prompt_declarations 注入到 prompt 组装管线，处理 block_id 去重
- [ ] T048 [US3] 为 prompt 桥接编写单元测试（instructions 注入、block_id 去重），在 `bridge/prompt_bridge.rs` 底部 `#[cfg(test)]` 模块
- [ ] T049 [US3] 确认 `cargo test -p astrcode-runtime-mcp` 通过

**Checkpoint**: MCP 服务器可注入 prompt 指令

---

## Phase 6: User Story 4 - 通过 MCP 使用服务器提供的资源 (Priority: P4)

**Goal**: 内置 ListMcpResources 和 ReadMcpResource 工具让 Agent 查询 MCP 资源

**Independent Test**: 连接一个暴露资源的 MCP 服务器，Agent 调用 ListMcpResources 列出资源，调用 ReadMcpResource 读取内容

### Implementation for User Story 4

- [ ] T050 [P] [US4] 实现 `crates/runtime-mcp/src/bridge/resource_tool.rs`：`ListMcpResourcesTool` 实现 Tool trait——遍历所有已连接服务器调用 `resources/list`，返回资源 URI、名称和描述
- [ ] T051 [P] [US4] 在 `crates/runtime-mcp/src/bridge/resource_tool.rs`：`ReadMcpResourceTool` 实现 Tool trait——指定服务器名和 URI 调用 `resources/read`，二进制内容持久化到磁盘并返回文件路径
- [ ] T052 [US4] 为 McpClient 实现 `list_resources()` 和 `read_resource()` 方法
- [ ] T053 [US4] 修改 McpConnectionManager：注册 ListMcpResources 和 ReadMcpResource 为内置工具
- [ ] T054 [US4] 为资源工具编写单元测试（列出资源、读取文本/二进制资源），在 `bridge/resource_tool.rs` 底部 `#[cfg(test)]` 模块
- [ ] T055 [US4] 确认 `cargo test -p astrcode-runtime-mcp` 通过

**Checkpoint**: MCP 资源可通过内置工具访问

---

## Phase 7: User Story 5 - 项目级 MCP 服务器审批 (Priority: P5)

**Goal**: 项目配置中的 MCP 服务器首次连接前需要用户审批

**Independent Test**: 在项目中创建 `.mcp.json`，验证首次启动时服务器处于 pending 状态，审批后才连接

### Implementation for User Story 5

- [ ] T056 [US5] 完善 `config/approval.rs`：审批状态通过 `McpSettingsStore` trait（定义在 `config/settings_port.rs`）持久化，runtime 在 bootstrap 时注入具体实现（读写本地 settings 文件），支持单服务器审批、全部批准、拒绝操作
- [ ] T057 [US5] 修改 McpConnectionManager.connect_all()：项目级服务器在连接前检查审批状态，pending 状态跳过连接并记录日志；提供 `pending_approval_servers()` 方法供 API 查询
- [ ] T058 [US5] 在 server 层新增 `crates/server/src/routes/mcp.rs`：`GET /api/mcp/status` 返回所有服务器状态（含 pending 的项目服务器），`POST /api/mcp/approve` 和 `POST /api/mcp/reject` 处理审批操作——server 层通过 RuntimeService 访问 McpConnectionManager，不直接依赖 runtime-mcp crate
- [ ] T059 [US5] 为审批流程编写单元测试（pending/approved/rejected 状态转换、McpSettingsStore trait mock），在 `config/approval.rs` 底部 `#[cfg(test)]` 模块
- [ ] T060 [US5] 确认 `cargo test -p astrcode-runtime-mcp` 通过

**Checkpoint**: 项目级 MCP 服务器需审批后才连接

---

## Phase 8: Polish & Cross-Cutting Concerns

**Purpose**: 跨 user story 的改进、集成测试和最终验证

- [ ] T061 [P] 实现 `crates/runtime-mcp/src/bridge/skill_bridge.rs`：MCP skill → SkillSpec 转换，注册到 SkillCatalog（优先级：builtin < mcp < plugin）
- [ ] T062 [P] 为 McpConnectionManager 实现完整的 list_changed 通知处理：收到 tools/list_changed 时清除工具缓存、重新获取工具列表、通过 CapabilityRouter 动态更新已注册的 invoker
- [ ] T063 审查所有文件行数不超过 800 行，超过则拆分模块
- [ ] T064 审查所有异步操作：无 unwrap/expect on locks、spawn 句柄均被管理、无 lock-held-across-await
- [ ] T065 审查所有关键操作的日志：连接/断开/重连/工具调用有结构化日志，错误级别正确，无静默忽略
- [ ] T066 确认 `cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test` 全量通过

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies - can start immediately
- **Foundational (Phase 2)**: Depends on Phase 1 completion - BLOCKS all user stories
- **US1 (Phase 3)**: Depends on Phase 2 - MVP
- **US2 (Phase 4)**: Depends on Phase 3 (需要 McpConnectionManager 基础实现)
- **US3 (Phase 5)**: Depends on Phase 3 (需要连接管理器)，US3 和 US4 可并行
- **US4 (Phase 6)**: Depends on Phase 3 (需要连接管理器)，US4 和 US3 可并行
- **US5 (Phase 7)**: Depends on Phase 3 (需要审批集成到连接流程)
- **Polish (Phase 8)**: Depends on all desired user stories

### User Story Dependencies

- **US1 (P1)**: 基础连接 + 工具桥接——所有其他 story 的前提
- **US2 (P2)**: 生命周期管理——依赖 US1 的 McpConnectionManager
- **US3 (P3)**: Prompt 注入——依赖 US1 的连接流程，与 US4 并行
- **US4 (P4)**: 资源访问——依赖 US1 的连接流程，与 US3 并行
- **US5 (P5)**: 审批流程——依赖 US1 的连接流程

### Within Each User Story

- 传输层/协议层实现 → 桥接层 → 管理器 → runtime 集成 → 测试验证
- 核心实现先于集成修改
- 单元测试与实现同文件

### Parallel Opportunities

- Phase 1 中 T002-T006 可并行（不同模块文件）
- Phase 3 中 T018/T019（bridge）和 T023-T025（config）可并行
- Phase 3 中 T028/T029（HTTP/SSE transport）可并行
- Phase 5 (US3) 和 Phase 6 (US4) 可完全并行
- Phase 8 中 T061/T062 可并行

---

## Parallel Example: User Story 1

```text
# 传输层（可并行）:
T028: StreamableHttpTransport in transport/http.rs
T029: SseTransport in transport/sse.rs

# 桥接层 + 配置（可并行，不同文件）:
T018: McpToolBridge in bridge/tool_bridge.rs
T019: bridge mod.rs
T023: McpConfigManager in config/loader.rs
T024: McpApprovalManager in config/approval.rs
T025: McpPolicyFilter in config/policy.rs

# 依赖前面的串行任务:
T020: McpConnectionManager（依赖 T018, T023）
T026: runtime_surface_assembler 集成（依赖 T020）
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup → crate 骨架就绪
2. Complete Phase 2: Foundational → 传输层 + 协议客户端就绪
3. Complete Phase 3: US1 → MCP 工具可连接、可注册、可调用
4. **STOP and VALIDATE**: 用真实 MCP 服务器测试端到端流程
5. 可交付 MVP

### Incremental Delivery

1. Setup + Foundational → 基础设施就绪
2. + US1 → MCP 工具可调用（MVP）
3. + US2 → 自动重连 + 热加载
4. + US3/US4（并行）→ Prompt 注入 + 资源访问
5. + US5 → 项目服务器审批
6. + Polish → 生产就绪

---

## Notes

- [P] tasks = different files, no dependencies
- [Story] label maps task to specific user story for traceability
- 每个 checkpoint 后运行 `cargo test -p astrcode-runtime-mcp` 验证
- runtime-mcp 单个文件不超过 800 行（宪法约束）
- 所有中文注释解释"为什么"和"做了什么"
- 最终全量验证：`cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test`
- runtime 集成使用 `Option<McpConnectionManager>` 保护——MCP 不可用时 runtime 照常启动
- settings 持久化通过 trait 抽象（`McpSettingsStore`），runtime-mcp 不直接读写 settings 文件
