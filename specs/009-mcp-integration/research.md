# Research: MCP Server 接入支持

**Feature**: 009-mcp-integration
**Date**: 2026-04-12

## 研究总结

基于对 Claude Code 源码 (`claude-code-sourcemap/restored-src/src/`) 中 MCP 实现的深度分析，结合 Astrcode 项目现有架构，形成以下技术决策。

---

## R1: 新 Crate 的定位与依赖方向

**决策**: 新增 `crates/runtime-mcp` crate，与 `plugin` crate 并行，位于同一架构层。

**理由**:
- 宪法 II (One Boundary, One Owner) 要求每个编译边界拥有且只拥有一类核心职责
- `plugin` crate 负责自定义插件进程管理（JSON-RPC 自定义协议），MCP 是标准化协议接入，职责不同
- 两者都实现 `CapabilityInvoker` trait（来自 `core`），注册到 `CapabilityRouter`（来自 `runtime-registry`），共享统一注册机制但不共享实现
- `runtime-mcp` 仅依赖 `core`，不依赖 `runtime`，符合宪法约束（runtime-...-loader 系列 MUST 依赖 core 而非 runtime）

**排除的替代方案**:
- 在 `plugin` crate 中添加 MCP 支持：违反宪法 II，MCP 和自定义插件是不同协议、不同职责
- 在 `runtime` crate 中直接实现：违反宪法约束（runtime 只做组合，不复制子 crate 逻辑）

---

## R2: MCP 传输协议选型

**决策**: 优先支持 stdio + Streamable HTTP，SSE 作为兼容回退。v1 不支持 WebSocket。

**理由**:
- Claude Code 源码支持 stdio/sse/http/ws/sdk/claudeai-proxy 等多种传输，但核心高频使用的是 stdio 和 HTTP
- MCP 规范 2025-03-26 已将 SSE 标记为废弃，推荐 Streamable HTTP
- 保留 SSE 回退以兼容现有生态中的旧 MCP 服务器
- WebSocket 增加实现复杂度但用户量较少，推迟到后续版本

**实现策略**:
- stdio: 使用 `tokio::process::Command` 启动子进程，stdin/stdout 做 JSON-RPC 传输
- Streamable HTTP: 使用 `reqwest` 发送 HTTP POST，接收 SSE 响应流
- SSE 回退: 检测到服务器返回 SSE 响应时自动切换

---

## R3: MCP 协议层实现策略

**决策**: 直接实现 MCP JSON-RPC 协议层，不依赖外部 MCP SDK。

**理由**:
- Rust 生态中暂无成熟的官方 MCP SDK（TypeScript 有 `@modelcontextprotocol/sdk`）
- MCP 协议本质是 JSON-RPC 2.0 + 少量标准化方法（initialize/tools/list/tools/call/prompts/*/resources/*）
- Astrcode 已有 `reqwest` + `tokio::process` + `serde_json` 依赖，可复用
- 自行实现协议层可以精确控制错误处理、取消信号和流式响应

**实现范围**:
- JSON-RPC 2.0 消息收发（request/response/notification）
- MCP 握手（`initialize` + `initialized` 通知）
- 工具发现与调用（`tools/list`、`tools/call`）
- Prompt 模板（`prompts/list`、`prompts/get`）
- 资源访问（`resources/list`、`resources/read`）
- 取消通知（`notifications/cancelled`）
- List change 通知（`notifications/tools/list_changed` 等）

---

## R4: 连接状态机设计

**决策**: 采用 5 状态连接模型（参考 Claude Code 的 `MCPServerConnection` 联合类型）。

**状态转换**:
```
Pending → Connecting → Connected ←→ Pending (reconnect)
                   ↘ Failed → Pending (retry)
                   ↘ NeedsAuth → Connected (after auth)
Connected → Disabled (user toggle)
Disabled → Pending (user toggle)
Failed → Disabled (max retries exceeded)
```

**理由**:
- Claude Code 验证了 5 状态模型足以覆盖所有 MCP 连接场景
- 区分 Failed 和 NeedsAuth 很重要：前者是系统错误，后者需要用户介入
- Disabled 状态让用户可以暂时禁用不需要的服务器而不删除配置

---

## R5: 工具桥接模式

**决策**: 创建 `McpToolBridge` 结构体实现 `Tool` trait，再通过 `ToolCapabilityInvoker` 包装为 `CapabilityInvoker`。

**理由**:
- Astrcode 已有 `Tool` → `ToolCapabilityInvoker` → `CapabilityRouter` 的注册链路
- 在 `Tool` 层桥接可以复用现有的 `ToolContext`、`ToolExecutionResult`、取消令牌等基础设施
- Claude Code 使用了相同的模式（MCPTool → Tool interface → registration）
- 工具命名使用 `mcp__{serverName}__{toolName}` 格式

**桥接映射**:
- MCP `inputSchema` → `ToolDefinition.parameters`（直接映射，都是 JSON Schema）
- MCP `annotations.readOnlyHint` → `ToolCapabilityMetadata` 的 `concurrency_safe`
- MCP `annotations.destructiveHint` → `SideEffectLevel`
- MCP 工具结果 → `ToolExecutionResult`（复用已有落盘机制）

---

## R6: 配置管理策略

**决策**: MCP 配置分为两层：
1. 项目级 `.mcp.json`（JSON 格式，与 Claude Code 生态兼容，可提交到版本控制）
2. 用户/本地配置在现有 settings 体系中

**理由**:
- `.mcp.json` 是 MCP 生态的事实标准（Claude Code、Cursor 等均使用此格式）
- 使用 JSON 而非 TOML 是为了与 MCP 生态兼容
- 项目级配置需要审批机制（安全考虑：stdio command 可执行任意命令）

**去重策略**:
- stdio 服务器按 `(command, args)` 签名去重
- 远程服务器按 URL 签名去重
- 高优先级配置覆盖低优先级：`user < project < local`

---

## R7: 与 runtime 门面的集成点

**决策**: MCP 作为 `PluginInitializer` 的并行路径，在 `runtime_surface_assembler.rs` 中统一组装。

**理由**:
- `runtime_surface_assembler.rs` 已经是能力面组装的统一入口
- `PluginInitializer` trait 是连接初始化的抽象，MCP 可以复用类似模式
- MCP 的贡献模型（capabilities + prompt_declarations + skills + hooks）与插件一致
- 不需要在 runtime 中新增"第二套业务实现层"（宪法约束）

**集成流程**:
1. bootstrap 阶段加载 MCP 配置
2. 通过 MCP 初始化器连接所有服务器
3. 将 MCP 工具、prompt、skill 注入到 `AssembledRuntimeSurface`
4. 与插件一样，MCP 服务器也作为 `ManagedRuntimeComponent` 管理生命周期

---

## R8: 取消与超时设计

**决策**:
- 工具调用取消：发送 `notifications/cancelled`，设置 30 秒等待上限，超时则强制断开连接
- 连接超时：握手阶段 30 秒超时
- 调用超时：单个 `tools/call` 请求 120 秒超时（默认，可配置）

**理由**:
- MCP 协议定义了 `notifications/cancelled` 用于取消进行中的请求
- Claude Code 的默认超时约 100 秒（但允许环境变量覆盖）
- 强制断开是必要的安全阀：不是所有 MCP 服务器都会响应取消通知
