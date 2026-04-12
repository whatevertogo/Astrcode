# Contract: McpClient

**Feature**: 009-mcp-integration
**Purpose**: MCP 协议层的客户端接口，封装所有 MCP 协议方法

## 接口签名

```rust
/// MCP 协议客户端。
///
/// 封装 MCP JSON-RPC 协议的所有标准化方法。
/// 持有传输层引用，负责消息构造和响应解析。
pub struct McpClient {
    transport: Arc<dyn McpTransport>,
    server_info: Option<McpServerInfo>,
    capabilities: Option<McpServerCapabilities>,
    instructions: Option<String>,
}

impl McpClient {
    /// 通过传输层创建并初始化 MCP 客户端。
    ///
    /// 执行完整的 MCP 握手流程：
    /// 1. 发送 `initialize` 请求
    /// 2. 验证协议版本兼容性
    /// 3. 接收服务器信息和能力声明
    /// 4. 发送 `initialized` 通知
    pub async fn connect(transport: Arc<dyn McpTransport>) -> Result<Self>;

    /// 请求服务器的能力列表。
    ///
    /// 仅在服务器声明了 `tools` 能力时调用。
    pub async fn list_tools(&self) -> Result<Vec<McpToolInfo>>;

    /// 调用服务器上的工具。
    ///
    /// 支持进度回调和取消信号。
    pub async fn call_tool(
        &self,
        tool_name: &str,
        arguments: Value,
        cancel: CancelToken,
    ) -> Result<McpToolResult>;

    /// 请求服务器的 prompt 模板列表。
    ///
    /// 仅在服务器声明了 `prompts` 能力时调用。
    pub async fn list_prompts(&self) -> Result<Vec<McpPromptInfo>>;

    /// 获取指定 prompt 模板的内容。
    pub async fn get_prompt(
        &self,
        prompt_name: &str,
        arguments: HashMap<String, String>,
    ) -> Result<Vec<McpPromptMessage>>;

    /// 请求服务器的资源列表。
    ///
    /// 仅在服务器声明了 `resources` 能力时调用。
    pub async fn list_resources(&self) -> Result<Vec<McpResourceInfo>>;

    /// 读取指定资源的内容。
    pub async fn read_resource(&self, uri: &str) -> Result<Vec<McpResourceContent>>;

    /// 发送取消通知。
    ///
    /// 请求服务器取消进行中的请求。不保证服务器会响应。
    pub async fn send_cancel(&self, request_id: &str, reason: Option<&str>) -> Result<()>;

    /// 注册 list_changed 通知处理器。
    ///
    /// 当服务器推送 `tools/list_changed` 等通知时调用回调。
    pub fn on_list_changed(&self, kind: McpListKind, handler: Box<dyn Fn() + Send + Sync>);

    /// 关闭客户端，发送关闭通知。
    pub async fn disconnect(self) -> Result<()>;
}
```

## MCP 握手请求/响应

```rust
/// `initialize` 请求参数
pub struct InitializeParams {
    pub protocol_version: String,       // "2025-03-26"
    pub client_info: ClientInfo,
    pub capabilities: ClientCapabilities,
}

/// `initialize` 响应
pub struct InitializeResult {
    pub protocol_version: String,
    pub server_info: McpServerInfo,
    pub capabilities: McpServerCapabilities,
    pub instructions: Option<String>,
}

/// 服务器能力声明
pub struct McpServerCapabilities {
    pub tools: Option<ToolsCapability>,       // 含 listChanged 标志
    pub prompts: Option<PromptsCapability>,    // 含 listChanged 标志
    pub resources: Option<ResourcesCapability>, // 含 subscribe, listChanged 标志
}
```

## 版本兼容性规则

- 客户端发送其支持的最高协议版本
- 服务器返回其支持的协议版本
- 如果服务器版本低于客户端最低兼容版本，连接失败
- 最低兼容版本：`2024-11-05`（MCP 初始版本）
