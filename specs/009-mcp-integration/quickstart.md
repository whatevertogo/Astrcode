# Quickstart: MCP Server 接入支持

**Feature**: 009-mcp-integration

## 5 分钟快速验证

### 1. 添加 MCP 服务器配置

在项目根目录创建 `.mcp.json`：

```json
{
  "mcpServers": {
    "echo": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-everything"]
    }
  }
}
```

### 2. 启动应用

应用启动后会自动：
1. 发现 `.mcp.json` 配置
2. 弹出审批对话框（首次）
3. 连接 MCP 服务器
4. 注册工具到能力路由

### 3. 在对话中使用

Agent 可以直接调用 MCP 工具：

```
用户: 帮我用 echo 工具测试一下
Agent: [调用 mcp__echo__echo 工具，获得结果]
```

## 新增 Crate 结构

```text
crates/runtime-mcp/
├── Cargo.toml
└── src/
    ├── lib.rs                    # 公共导出
    ├── transport/                # 传输层
    │   ├── mod.rs                # McpTransport trait
    │   ├── stdio.rs              # StdioTransport
    │   ├── http.rs               # StreamableHttpTransport
    │   └── sse.rs                # SseTransport（兼容回退）
    ├── protocol/                 # MCP 协议层
    │   ├── mod.rs                # JSON-RPC 消息类型
    │   ├── client.rs             # McpClient（握手、工具调用等）
    │   ├── types.rs              # DTO: ToolInfo, PromptInfo, ResourceInfo 等
    │   └── error.rs              # MCP 协议错误类型
    ├── bridge/                   # Astrcode 桥接层
    │   ├── mod.rs
    │   ├── tool_bridge.rs        # McpToolBridge (impl Tool)
    │   ├── prompt_bridge.rs      # Prompt 声明转换
    │   ├── resource_tool.rs      # ListMcpResources + ReadMcpResource 工具
    │   └── skill_bridge.rs       # MCP skill → SkillSpec 转换
    ├── config/                   # 配置管理
    │   ├── mod.rs
    │   ├── loader.rs             # 多作用域加载 + 去重
    │   ├── approval.rs           # 审批状态管理
    │   └── policy.rs             # 策略过滤
    ├── connection.rs             # McpConnection 状态机
    ├── manager.rs                # McpConnectionManager（生命周期管理）
    └── hot_reload.rs             # 热加载逻辑
```

## 依赖方向

```text
core (CapabilityInvoker, Tool, ManagedRuntimeComponent)
  ↑
runtime-mcp (McpClient, McpConnectionManager, McpToolBridge)
  ↑
runtime (RuntimeService 门面，通过 assembler 组合)
  ↑
server (HTTP API 暴露 MCP 状态)
```

## 验证命令

```bash
# 编译检查
cargo build -p astrcode-runtime-mcp

# 单元测试
cargo test -p astrcode-runtime-mcp

# 集成测试（需要本地 MCP 服务器）
cargo test -p astrcode-runtime-mcp --features integration

# 全量验证
cargo fmt --all --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test
```
