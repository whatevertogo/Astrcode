# Contract: MCP 配置与审批

**Feature**: 009-mcp-integration
**Purpose**: MCP 配置加载、去重和审批流程的接口定义

## 配置加载

```rust
/// MCP 配置管理器。
///
/// 负责从多个作用域加载配置、去重和环境变量展开。
pub struct McpConfigManager {
    user_configs: HashMap<String, McpServerConfig>,
    project_configs: HashMap<String, McpServerConfig>,
    local_configs: HashMap<String, McpServerConfig>,
}

/// 配置文件路径定位。
pub struct McpConfigPaths {
    /// 用户全局配置目录（如 `~/.astrcode/`）。
    pub user_config_dir: PathBuf,
    /// 项目根目录（用于查找 `.mcp.json`，从 CWD 向上遍历）。
    pub project_dir: PathBuf,
    /// 项目本地私有配置目录（如 `.astrcode/`）。
    pub local_config_dir: PathBuf,
}

impl McpConfigManager {
    /// 从所有作用域加载并合并配置。
    ///
    /// 优先级：user < project < local
    /// 同签名时高优先级覆盖低优先级。
    pub async fn load_all(paths: McpConfigPaths) -> Result<Self>;

    /// 检测配置是否有变更（用于热加载）。
    pub async fn has_changes(&self) -> bool;

    /// 获取合并后的配置列表。
    pub fn merged_configs(&self) -> Vec<McpServerConfig>;

    /// 根据项目路径加载 `.mcp.json`。
    ///
    /// 从 CWD 向上遍历查找，就近优先。
    fn load_project_config(working_dir: &Path) -> Result<HashMap<String, McpServerConfig>>;

    /// 展开配置中的环境变量。
    ///
    /// `${VAR}` 格式，缺失的变量记录错误并返回 Err，
    /// 调用方据此将该服务器标记为 failed。
    fn expand_env_vars(config: &mut McpServerConfig) -> Result<Vec<EnvVarWarning>>;

    /// 基于签名去重。
    ///
    /// stdio 按 `command:args` 签名，远程按 URL 签名。
    fn dedup_configs(configs: &mut HashMap<String, McpServerConfig>) -> Vec<DedupWarning>;
}
```

## 审批流程

```rust
/// 项目级 MCP 服务器的审批管理。
pub struct McpApprovalManager {
    approval_store: McpApprovalStore,
}

impl McpApprovalManager {
    /// 检查服务器审批状态。
    pub fn get_status(&self, server_name: &str) -> McpApprovalStatus;

    /// 批准单个服务器。
    pub fn approve(&mut self, server_name: &str) -> Result<()>;

    /// 批准所有项目服务器。
    pub fn approve_all(&mut self) -> Result<()>;

    /// 拒绝服务器。
    pub fn reject(&mut self, server_name: &str) -> Result<()>;

    /// 获取所有待审批的服务器列表。
    pub fn pending_servers(&self) -> Vec<String>;
}

/// 审批状态持久化接口。
///
/// 存储在本地 settings 中，不随项目版本控制。
pub trait McpApprovalStore: Send + Sync {
    fn load(&self) -> Result<McpApprovalData>;
    fn save(&self, data: &McpApprovalData) -> Result<()>;
}

/// 审批数据。
pub struct McpApprovalData {
    /// 已批准的服务器列表。
    pub approved_servers: HashSet<String>,
    /// 已拒绝的服务器列表。
    pub rejected_servers: HashSet<String>,
    /// 是否已全局批准所有项目服务器。
    pub approve_all_project: bool,
}
```

## 策略过滤

```rust
/// MCP 策略过滤器。
///
/// 基于允许/拒绝列表控制哪些 MCP 服务器可以被连接。
pub struct McpPolicyFilter {
    allowed: Option<Vec<McpServerPattern>>,
    denied: Vec<McpServerPattern>,
}

/// 服务器匹配模式。
pub enum McpServerPattern {
    Name { server_name: String },
    Command { command: Vec<String> },
    Url { pattern: String },  // 支持通配符
}

impl McpPolicyFilter {
    /// 检查服务器是否被策略允许。
    ///
    /// 拒绝列表优先于允许列表。
    /// 无允许列表 = 允许所有。
    /// 空允许列表 = 阻止所有。
    pub fn is_allowed(&self, config: &McpServerConfig) -> bool;

    /// 从 settings 加载策略配置。
    pub fn from_settings() -> Result<Self>;
}
```
