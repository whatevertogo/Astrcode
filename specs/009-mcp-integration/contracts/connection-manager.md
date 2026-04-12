# Contract: McpConnectionManager

**Feature**: 009-mcp-integration
**Purpose**: MCP 服务器连接生命周期管理器，统一管理所有 MCP 连接

## 接口签名

```rust
/// MCP 连接管理器。
///
/// 负责所有 MCP 服务器的连接生命周期，包括：
/// - 批量连接与错误隔离
/// - 自动重连（仅远程传输）
/// - 热加载（配置变更时增减连接）
/// - 连接状态追踪
///
/// 实现 `ManagedRuntimeComponent` 以便 runtime 统一管理关闭顺序。
pub struct McpConnectionManager {
    registry: Arc<RwLock<McpServerRegistry>>,
    config_manager: McpConfigManager,
    rejoin_handles: HashMap<String, JoinHandle<()>>,
}

impl McpConnectionManager {
    /// 从配置创建连接管理器。
    pub fn new(config_manager: McpConfigManager) -> Self;

    /// 批量连接所有已声明的 MCP 服务器。
    ///
    /// 本地服务器并发度 ≤ 3，远程服务器并发度 ≤ 10。
    /// 单个服务器失败不阻塞其他服务器。
    ///
    /// 返回连接贡献（capabilities + prompt_declarations + skills），
    /// 供 runtime surface assembler 注册。
    pub async fn connect_all(&self) -> McpConnectionResults;

    /// 连接单个 MCP 服务器。
    pub async fn connect_one(&self, name: &str) -> Result<McpSingleConnectionResult>;

    /// 断开单个 MCP 服务器。
    ///
    /// 等待正在进行的工具调用完成（超时 30 秒），然后断开。
    pub async fn disconnect_one(&self, name: &str) -> Result<()>;

    /// 热加载：检测配置变更，增减连接。
    pub async fn reload_config(&self) -> McpReloadResult;

    /// 获取所有已连接服务器的工具（已包装为 CapabilityInvoker）。
    pub fn connected_invokers(&self) -> Vec<Arc<dyn CapabilityInvoker>>;

    /// 获取所有已连接服务器的 prompt 声明。
    pub fn connected_prompt_declarations(&self) -> Vec<PromptDeclaration>;

    /// 获取指定服务器的连接状态。
    pub fn connection_status(&self, name: &str) -> McpConnectionState;

    /// 获取所有服务器的状态摘要（供 API 查询）。
    pub fn all_status(&self) -> Vec<McpServerStatus>;
}

/// 连接结果，供 runtime surface assembler 消费。
pub struct McpConnectionResults {
    pub tool_invokers: Vec<Arc<dyn CapabilityInvoker>>,
    pub prompt_declarations: Vec<PromptDeclaration>,
    pub skills: Vec<SkillSpec>,
    pub managed_components: Vec<Arc<dyn ManagedRuntimeComponent>>,
    pub server_statuses: Vec<McpServerStatus>,
    pub warnings: Vec<String>,  // 冲突警告、环境变量缺失等
}

/// 单个服务器的连接结果。
pub struct McpSingleConnectionResult {
    pub tool_invokers: Vec<Arc<dyn CapabilityInvoker>>,
    pub prompt_declarations: Vec<PromptDeclaration>,
    pub skills: Vec<SkillSpec>,
    pub status: McpConnectionState,
}

/// 热加载结果。
pub struct McpReloadResult {
    pub added: Vec<String>,      // 新增的服务器名
    pub removed: Vec<String>,    // 移除的服务器名
    pub unchanged: Vec<String>,  // 未变化的服务器名
    pub failed: Vec<(String, String)>, // 失败的服务器名和错误
}
```

## 与 Runtime 的集成点

```rust
/// McpConnectionManager 实现 ManagedRuntimeComponent。
#[async_trait]
impl ManagedRuntimeComponent for McpConnectionManager {
    fn component_name(&self) -> String {
        "mcp-connection-manager".to_string()
    }

    async fn shutdown_component(&self) -> Result<()> {
        // 关闭所有连接，取消所有重连任务
        // 等待正在进行的工具调用完成（总超时 60 秒）
    }
}
```

## 重连策略

```rust
/// 重连配置
pub struct ReconnectPolicy {
    pub max_attempts: u32,         // 默认 5
    pub initial_backoff_ms: u64,   // 默认 1000
    pub max_backoff_ms: u64,       // 默认 30000
    pub multiplier: f64,           // 默认 2.0（指数退避）
}

/// 重连条件：
/// - 仅远程传输（Streamable HTTP、SSE）
/// - 服务器未被用户禁用
/// - 未超过最大重连次数
/// - stdio 服务器断开后不自动重启（进程崩溃视为需用户干预）
```
