# Data Model: MCP Server 接入支持

**Feature**: 009-mcp-integration
**Date**: 2026-04-12

## 实体关系概览

```text
McpServerConfig ──1:1──→ McpConnection
    │                      │
    │                      ├── 1:N → McpToolBridge (implements Tool)
    │                      ├── 0:N → McpPromptTemplate (注册为 Command)
    │                      └── 0:N → McpResource (通过 ListMcpResources 工具暴露)
    │
    └── McpConfigScope (enum: User/Project/Local)

McpConnection ──state──→ McpConnectionState (enum: Pending/Connecting/Connected/Failed/NeedsAuth/Disabled)

McpServerRegistry ──1:N──→ McpManagedServer
    │
    └── 持有所有 McpConnection 实例，负责生命周期管理

McpConfigManager ──1:1──→ McpServerRegistry
    │
    └── 负责配置加载、热加载、去重
```

## 核心实体

### McpServerConfig

用户声明的 MCP 服务器配置，从 `.mcp.json` 或 settings 解析而来。

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| name | String | 是 | 服务器唯一标识，仅允许 `[a-zA-Z0-9_-]` |
| transport | McpTransportConfig | 是 | 传输配置（见子类型） |
| scope | McpConfigScope | 是 | 配置来源作用域 |
| enabled | bool | 否 | 默认 true，false 时跳过连接 |
| timeout_secs | u64 | 否 | 单次请求超时，默认 120 |
| init_timeout_secs | u64 | 否 | 握手超时，默认 30 |
| max_reconnect_attempts | u32 | 否 | 最大重连次数，默认 5（仅远程） |

**McpTransportConfig** (联合类型):

| 变体 | 字段 | 说明 |
|------|------|------|
| Stdio | command: String, args: Vec\<String\>, env: HashMap\<String, String\> | 本地进程传输 |
| StreamableHttp | url: String, headers: HashMap\<String, String\>, oauth: Option\<McpOAuthConfig\> | Streamable HTTP 远程传输 |
| Sse | url: String, headers: HashMap\<String, String\>, oauth: Option\<McpOAuthConfig\> | SSE 兼容回退 |

**McpOAuthConfig**:

| 字段 | 类型 | 说明 |
|------|------|------|
| client_id | Option\<String\> | 预注册的 client_id |
| callback_port | Option\<u16\> | OAuth 回调端口 |
| auth_server_metadata_url | Option\<String\> | 授权服务器元数据 URL |

### McpConnection

与 MCP 服务器的活动连接，管理传输通道和生命周期。

| 字段 | 类型 | 说明 |
|------|------|------|
| name | String | 服务器名称（与 Config.name 一致） |
| state | McpConnectionState | 当前连接状态 |
| config | McpServerConfig | 原始配置引用 |
| server_info | Option\<McpServerInfo\> | 握手后获取的服务器信息 |
| capabilities | Option\<McpServerCapabilities\> | 服务器支持的能力集 |
| instructions | Option\<String\> | 服务器提供的 prompt 指令 |
| transport_handle | Option\<Arc\<dyn McpTransport\>\> | 传输通道句柄 |
| reconnect_attempt | u32 | 当前重连尝试次数 |
| cleanup_fn | Option\<Box\<dyn Fn() -> Result\<()\> + Send + Sync\>\> | 清理函数（关闭进程/连接） |

**McpConnectionState** (状态枚举):

```
Pending     → 初始状态，等待连接
Connecting  → 正在握手
Connected   → 已连接，可调用
Failed      → 连接或调用失败，含错误信息
NeedsAuth   → 远程服务器需要认证
Disabled    → 用户手动禁用
```

**状态转换规则**:
- Pending → Connecting → Connected（正常流程）
- Connecting → Failed（握手失败）
- Connecting → NeedsAuth（401/403 响应）
- Connected → Pending（连接断开，触发重连，仅远程）
- Failed → Pending（重试）
- Connected → Disabled（用户操作）
- Disabled → Pending（用户操作）
- Failed → Disabled（超过最大重连次数）

### McpToolBridge

将 MCP 工具桥接为 Astrcode Tool trait 的适配器。

| 字段 | 类型 | 说明 |
|------|------|------|
| server_name | String | 所属 MCP 服务器名称 |
| tool_name | String | MCP 服务器声明的原始工具名 |
| fully_qualified_name | String | `mcp__{server}__{tool}` 格式 |
| description | String | 工具描述 |
| input_schema | Value | JSON Schema（直接来自 MCP） |
| annotations | McpToolAnnotations | 工具能力提示 |

**McpToolAnnotations**:

| 字段 | 类型 | 默认 | 映射到 |
|------|------|------|--------|
| read_only_hint | bool | false | `ToolCapabilityMetadata.concurrency_safe` |
| destructive_hint | bool | false | `SideEffectLevel` |
| open_world_hint | bool | false | `PermissionHint` |

### McpServerRegistry

所有 MCP 服务器的注册表和生命周期管理器。

| 字段 | 类型 | 说明 |
|------|------|------|
| servers | HashMap\<String, McpManagedServer\> | 所有管理的服务器 |
| config_manager | McpConfigManager | 配置加载和热加载 |

**McpManagedServer**:

| 字段 | 类型 | 说明 |
|------|------|------|
| config | McpServerConfig | 原始配置 |
| connection | McpConnection | 当前连接状态 |
| tools | Vec\<McpToolBridge\> | 已发现的工具列表 |
| prompt_templates | Vec\<McpPromptTemplate\> | 已发现的 prompt 模板 |
| resources | Vec\<McpResource\> | 已发现的资源列表 |
| approval_status | McpApprovalStatus | 审批状态（仅项目级配置） |

### McpApprovalStatus

项目级 MCP 服务器的审批状态。

| 值 | 说明 |
|----|------|
| Pending | 等待用户审批 |
| Approved | 已批准，可连接 |
| Rejected | 已拒绝，跳过连接 |

## 配置文件格式

### 项目级 `.mcp.json`

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path"],
      "env": { "NODE_ENV": "production" }
    },
    "github": {
      "type": "http",
      "url": "https://api.github.com/mcp",
      "headers": {
        "Authorization": "Bearer ${GITHUB_TOKEN}"
      }
    },
    "legacy-server": {
      "type": "sse",
      "url": "https://legacy.example.com/sse"
    }
  }
}
```

### 用户/本地配置（JSON settings 体系）

沿用项目现有的 `~/.astrcode/config.json` 和 `.astrcode/config.json` 格式，在 Config 结构中新增 `mcp` 字段：

```json
{
  "version": "1",
  "activeProfile": "deepseek",
  "activeModel": "deepseek-chat",
  "profiles": [],
  "mcp": {
    "servers": {
      "filesystem": {
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path"]
      },
      "github": {
        "type": "http",
        "url": "https://api.github.com/mcp",
        "headers": {
          "Authorization": "Bearer ${GITHUB_TOKEN}"
        }
      }
    },
    "policy": {
      "deniedServers": ["dangerous-server"]
    }
  }
}
```

## 签名去重规则

| 传输类型 | 签名公式 | 示例 |
|----------|----------|------|
| Stdio | `stdio:{command}:{args_json}` | `stdio:npx:["-y","@mcp/server"]` |
| StreamableHttp | `url:{url}` | `url:https://api.github.com/mcp` |
| Sse | `url:{url}` | `url:https://legacy.example.com/sse` |

优先级：`user < project < local`，同签名时高优先级覆盖低优先级。
