# Implementation Plan: MCP Server 接入支持

**Branch**: `009-mcp-integration` | **Date**: 2026-04-12 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `/specs/009-mcp-integration/spec.md`

## Summary

为 Astrcode 项目接入 MCP (Model Context Protocol) 服务器支持，允许用户在配置文件中声明 MCP 服务器，系统自动连接并将 MCP 工具注册到能力路由中。新增 `runtime-mcp` crate（与 `plugin` crate 并行），实现 MCP 协议传输层、客户端层、工具桥接层和连接生命周期管理。设计参考了 Claude Code 源码中经过验证的连接状态机、指数退避重连、差异化并发策略、配置去重和审批流程等模式。

## Technical Context

**Language/Version**: Rust 2021 Edition
**Primary Dependencies**: tokio（异步运行时 + 子进程管理）、reqwest（HTTP 传输）、serde/serde_json（JSON-RPC 序列化）、astrcode-core（Tool/CapabilityInvoker trait）
**Storage**: 文件系统（`.mcp.json` 配置、本地 settings 审批状态、工具结果持久化复用已有机制）
**Testing**: cargo test（单元 + 集成测试）
**Target Platform**: 跨平台桌面应用（Windows/macOS/Linux）
**Project Type**: 桌面应用后端（crate 库）
**Performance Goals**: 10 个 MCP 服务器并行连接，工具调用延迟不超过直连 1.2 倍
**Constraints**: runtime-mcp 仅依赖 core，不依赖 runtime；单个文件不超过 800 行
**Scale/Scope**: 预计 1-20 个 MCP 服务器，每个服务器 1-50 个工具

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

- **Durable Truth First**: ✅ MCP 连接是 live 状态，不涉及 durable 事件。工具调用结果通过已有的 `ToolExecutionResult` 落盘。MCP 工具的注册信息是 runtime 组装时的瞬态数据，不作为 durable truth。
- **One Boundary, One Owner**: ✅ 新增 `runtime-mcp` crate，职责单一：MCP 协议连接和工具桥接。与 `plugin` crate（自定义插件协议）并行，不重叠。MCP 的贡献（tools/prompts/skills）通过 `RuntimeSurfaceContribution` 统一注入，不创建第二套注册路径。
- **Protocol Purity, Projection Fidelity**: ✅ MCP 相关 DTO 定义在 `runtime-mcp` 内部（非 protocol crate），因为 MCP 是 runtime 实现细节而非对外协议。server 层仅暴露 MCP 状态查询 API。
- **Ownership Over Storage Mode**: ✅ 连接所有权归 `McpConnectionManager`，存储模式（进程/网络）与所有权分离。取消链路通过 `CancelToken` 显式传递。
- **Explicit Migrations, Verifiable Refactors**: ✅ 新增 crate，不涉及现有 API/事件/契约变更。`runtime_surface_assembler.rs` 需要扩展以集成 MCP，但接口不变（新增贡献来源）。
- **Runtime Robustness**: ✅ 所有异步操作有取消机制（`CancelToken`）。重连任务持有 `JoinHandle`，关闭时有序取消。不在持锁状态下 await。使用 `tokio::select!` 处理超时。
- **Observability & Error Visibility**: ✅ 关键操作（连接、断开、重连、工具调用）有结构化日志。错误级别语义一致。工具名冲突时记录明确警告（含两个来源标识）。

**Post-Phase 1 Re-check**: 所有设计决策与宪法一致。`runtime-mcp` 依赖 `core` 而非 `runtime`，符合编译隔离约束。

## Project Structure

### Documentation (this feature)

```text
specs/009-mcp-integration/
├── plan.md                          # 本文件
├── research.md                      # 技术决策研究
├── data-model.md                    # 数据模型
├── quickstart.md                    # 快速上手
├── contracts/
│   ├── transport.md                 # McpTransport trait 契约
│   ├── client.md                    # McpClient 契约
│   ├── tool-bridge.md               # McpToolBridge 契约（结果映射 + CancelToken 流转）
│   ├── connection-manager.md        # McpConnectionManager 契约
│   └── config-and-approval.md       # 配置与审批契约
├── checklists/
│   └── requirements.md              # 规格质量检查
├── spec.md                          # 功能规格
└── tasks.md                         # 任务分解（Phase 2，/speckit.tasks 生成）
```

### Source Code (repository root)

```text
crates/runtime-mcp/                   # 新增 crate
├── Cargo.toml
└── src/
    ├── lib.rs                        # 公共导出
    ├── transport/
    │   ├── mod.rs                    # McpTransport trait 定义
    │   ├── stdio.rs                  # StdioTransport: 子进程 stdin/stdout
    │   ├── http.rs                   # StreamableHttpTransport: HTTP POST + SSE
    │   └── sse.rs                    # SseTransport: SSE 兼容回退
    ├── protocol/
    │   ├── mod.rs                    # JSON-RPC 消息类型
    │   ├── client.rs                 # McpClient: 握手、工具调用
    │   ├── types.rs                  # DTO: ToolInfo, PromptInfo 等
    │   └── error.rs                  # 协议错误类型
    ├── bridge/
    │   ├── mod.rs
    │   ├── tool_bridge.rs            # McpToolBridge: impl Tool
    │   ├── prompt_bridge.rs          # prompt 声明 → PromptDeclaration
    │   ├── resource_tool.rs          # ListMcpResources + ReadMcpResource
    │   └── skill_bridge.rs           # MCP skill → SkillSpec
    ├── config/
    │   ├── mod.rs
    │   ├── loader.rs                 # 多作用域加载 + 去重
    │   ├── approval.rs               # 审批状态管理
    │   └── policy.rs                 # 策略过滤
    ├── connection.rs                 # McpConnection 状态机
    ├── manager.rs                    # McpConnectionManager
    └── hot_reload.rs                 # 配置文件监听 + 热加载

crates/runtime/src/                   # 修改
├── runtime_surface_assembler.rs      # 扩展: MCP 初始化路径
└── bootstrap.rs                      # 扩展: 加载 MCP 配置

crates/server/src/                    # 修改
└── routes/                           # 新增: MCP 状态 API
```

**Structure Decision**: 新增 `runtime-mcp` crate 与 `plugin` 并行。仅修改 `runtime`（组装集成）和 `server`（状态 API），不修改 `core` 或 `protocol`。

## Implementation Phases

### Phase A: 基础设施（传输层 + 协议层）

**目标**: 实现 MCP 传输抽象和 JSON-RPC 协议客户端，可独立测试。

1. 创建 `crates/runtime-mcp` crate 骨架
2. 实现 `McpTransport` trait 和 `StdioTransport`
3. 实现 JSON-RPC 2.0 消息类型
4. 实现 `McpClient`（握手 + tools/list + tools/call）
5. 单元测试：使用 mock 传输验证协议流程

### Phase B: 工具桥接 + 连接管理

**目标**: MCP 工具可注册到 CapabilityRouter，连接生命周期可管理。

1. 实现 `McpToolBridge`（impl `Tool` + 通过 `ToolCapabilityInvoker` 注册）
2. 实现 `McpConnection` 状态机
3. 实现 `McpConnectionManager`（批量连接、错误隔离、重连策略）
4. 实现 `StreamableHttpTransport`
5. 实现 `SseTransport`（兼容回退）
6. 集成测试：连接真实 MCP 服务器并调用工具

### Phase C: 配置管理 + 审批

**目标**: 支持从配置文件加载 MCP 服务器，实现审批和安全策略。

1. 实现 `McpConfigManager`（多作用域加载、环境变量展开、签名去重）
2. 实现 `.mcp.json` 解析
3. 实现 `McpApprovalManager`（审批状态持久化）
4. 实现 `McpPolicyFilter`（允许/拒绝列表）
5. 实现 `ListMcpResources` 和 `ReadMcpResource` 内置工具
6. 集成测试：配置热加载、审批流程

### Phase D: Runtime 集成 + Prompt 注入

**目标**: MCP 与 runtime 门面完全集成，prompt 指令注入到组装管线。

1. 扩展 `runtime_surface_assembler.rs`：集成 MCP 初始化路径
2. 扩展 `bootstrap.rs`：加载 MCP 配置并启动连接管理器
3. 实现 prompt 声明桥接（MCP instructions → `PromptDeclaration`）
4. 实现 skill 桥接（MCP skill → `SkillSpec`）
5. 实现 list_changed 通知处理（动态更新工具列表）
6. 实现 server 层 MCP 状态 API
7. 全链路集成测试

### Phase E: 热加载 + 资源 + 前端展示

**目标**: 完善热加载机制、资源工具和前端状态展示。

1. 实现配置文件监听和热加载
2. 完善 `ReadMcpResource` 工具（二进制内容持久化）
3. 前端 MCP 服务器状态展示
4. 前端审批对话框
5. 端到端测试

## Complexity Tracking

| 宪法相关点 | 处理方式 |
|-----------|---------|
| 新增 crate 边界 | `runtime-mcp` 职责单一，仅依赖 `core`，不依赖 `runtime` |
| runtime 门面文件行数 | `runtime_surface_assembler.rs` 扩展时如超 800 行需拆分 |
| 不创建第二套业务实现 | MCP 通过已有 `RuntimeSurfaceContribution` 注入，不新建注册路径 |
| 不向后兼容 | 新增功能，无兼容性负担 |
