//! 能力描述符与调用上下文
//!
//! 定义插件系统核心的元数据描述结构，是 host 与插件之间
//! 能力注册、路由、策略决策的基础协议。
//!
//! ## 主要类型
//!
//! - **PeerDescriptor**: 通信对等方的身份信息（ID、角色、版本、支持的 profile）
//! - **CapabilityDescriptor**: 能力的完整描述（名称、类型、schema、权限、副作用级别等）
//! - **CapabilityKind**: 能力类型的强类型包装，避免拼写错误导致路由失败
//! - **HandlerDescriptor**: 事件处理器的描述（触发条件、过滤规则）
//! - **InvocationContext**: 调用时的上下文（调用方、工作区、预算限制等）
//! - **CapabilityDescriptorBuilder**: 构建器模式，用于安全地构造能力描述符
//!
//! ## 设计原则
//!
//! - 能力描述符在插件握手时由插件发送给 host，host 据此进行路由和策略决策
//! - `CapabilityKind` 虽然是字符串包装，但提供了强类型的构造函数（`tool()`, `agent()` 等）
//! - Builder 在 `build()` 时执行完整校验，确保描述符的完整性
//! - 所有字段都有明确的默认值和 serde 注解，保证序列化兼容性

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// 通信对等方的角色类型。
///
/// 用于握手阶段标识 peer 的身份，影响能力路由和权限策略。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PeerRole {
    /// 核心运行时（host 侧）
    Core,
    /// 插件进程
    Plugin,
    /// 运行时服务
    Runtime,
    /// 工作进程
    Worker,
    /// 监督进程
    Supervisor,
}

/// 通信对等方的描述信息。
///
/// 在握手阶段交换，用于标识 peer 的身份、版本和支持的能力 profile。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PeerDescriptor {
    /// peer 的唯一标识
    pub id: String,
    /// peer 的显示名称
    pub name: String,
    /// peer 的角色类型
    pub role: PeerRole,
    /// peer 的版本号
    pub version: String,
    /// 此 peer 支持的 profile 列表
    #[serde(default)]
    pub supported_profiles: Vec<String>,
    /// 扩展元数据
    #[serde(default)]
    pub metadata: Value,
}

/// 能力类型的强类型包装。
///
/// 虽然底层是字符串，但通过此类型可以避免拼写错误导致的路由失败。
/// 提供了常见能力类型的构造函数（`tool()`, `agent()`, `context_provider()` 等），
/// 同时也支持通过 `new()` 创建用户自定义类型。
///
/// ## 设计动机
///
/// 此类型帮助 host 和插件进行路由、策略和展示决策。
/// 它不应被视为在 `{descriptor, invoke}` 之上叠加的第二调用协议——
/// 能力传输本身仍然是描述符 + 调用两阶段模型。
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct CapabilityKind(String);

impl CapabilityKind {
    /// 从字符串创建能力类型，自动去除首尾空白。
    pub fn new(kind: impl Into<String>) -> Self {
        Self(kind.into().trim().to_string())
    }

    /// 工具类型——可被 Agent 调用的功能。
    pub fn tool() -> Self {
        Self::new("tool")
    }

    /// Agent 类型——可自主执行多步操作的代理能力。
    pub fn agent() -> Self {
        Self::new("agent")
    }

    /// 上下文提供者——为 Agent 提供额外上下文信息。
    pub fn context_provider() -> Self {
        Self::new("context_provider")
    }

    /// 记忆提供者——为 Agent 提供持久化记忆能力。
    pub fn memory_provider() -> Self {
        Self::new("memory_provider")
    }

    /// 策略钩子——在 Agent 决策流程中插入策略检查。
    pub fn policy_hook() -> Self {
        Self::new("policy_hook")
    }

    /// 渲染器——负责将内容渲染为特定展示格式。
    pub fn renderer() -> Self {
        Self::new("renderer")
    }

    /// 资源——提供可被 Agent 访问的资源。
    pub fn resource() -> Self {
        Self::new("resource")
    }

    /// Prompt 模板——可被 Agent 使用的预定义 prompt。
    pub fn prompt() -> Self {
        Self::new("prompt")
    }

    /// 获取底层字符串引用。
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// 判断是否为工具类型。
    pub fn is_tool(&self) -> bool {
        self.as_str() == "tool"
    }
}

impl Default for CapabilityKind {
    fn default() -> Self {
        Self::tool()
    }
}

impl From<&str> for CapabilityKind {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for CapabilityKind {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for CapabilityKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self::new(String::deserialize(deserializer)?))
    }
}

impl fmt::Display for CapabilityKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 副作用级别。
///
/// 用于描述能力执行时对外部环境的影响程度，host 可据此进行权限策略决策。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectLevel {
    /// 无副作用（纯读取）
    #[default]
    None,
    /// 仅影响本地状态（如内存缓存）
    Local,
    /// 影响工作区（如修改文件）
    Workspace,
    /// 影响外部环境（如网络请求、系统命令）
    External,
}

/// 能力稳定性级别。
///
/// 用于前端展示和策略决策，标记能力是否处于实验阶段或已废弃。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum StabilityLevel {
    /// 实验性功能，API 可能变更
    Experimental,
    /// 稳定版本
    #[default]
    Stable,
    /// 已废弃，建议使用替代方案
    Deprecated,
}

/// 权限提示。
///
/// 描述能力执行时需要的权限，`rationale` 用于向用户解释为什么需要此权限。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PermissionHint {
    /// 权限名称
    pub name: String,
    /// 请求此权限的理由（向用户展示）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

/// 能力的完整描述符。
///
/// 这是插件协议的核心数据结构，在握手阶段由插件发送给 host。
/// Host 据此进行能力注册、路由配置、策略决策和前端展示。
///
/// ## 字段说明
///
/// - `kind`: 能力类型，影响路由和展示（如 tool 类型会展示为工具调用 UI）
/// - `input_schema` / `output_schema`: JSON Schema 格式的输入输出定义
/// - `streaming`: 是否支持流式输出（通过 `EventMessage` 返回中间结果）
/// - `concurrency_safe`: 是否可安全并发调用
/// - `compact_clearable`: 在上下文压缩时是否可以被清除
/// - `profiles`: 此能力可用的 profile 列表
/// - `side_effect`: 副作用级别，用于权限策略
/// - `stability`: 稳定性级别，用于前端展示
///
/// ## 设计注意
///
/// Host 可能将某些 kind 投影到特定展示面（如 tool-call UI），
/// 但能力传输本身仍然是 descriptor + invoke 两阶段模型。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDescriptor {
    /// 能力名称，支持点号命名空间（如 `tool.read_file`）
    pub name: String,
    /// 能力类型，影响路由和展示
    #[serde(default)]
    pub kind: CapabilityKind,
    /// 能力描述
    pub description: String,
    /// 输入参数的 JSON Schema
    pub input_schema: Value,
    /// 输出结果的 JSON Schema
    pub output_schema: Value,
    /// 是否支持流式输出
    #[serde(default)]
    pub streaming: bool,
    /// 是否可安全并发调用
    #[serde(default, skip_serializing_if = "is_false")]
    pub concurrency_safe: bool,
    /// 在上下文压缩时是否可被清除
    #[serde(default, skip_serializing_if = "is_false")]
    pub compact_clearable: bool,
    /// 此能力可用的 profile 列表
    #[serde(default)]
    pub profiles: Vec<String>,
    /// 能力标签，用于分类和搜索
    #[serde(default)]
    pub tags: Vec<String>,
    /// 需要的权限列表
    #[serde(default)]
    pub permissions: Vec<PermissionHint>,
    /// 副作用级别
    #[serde(default)]
    pub side_effect: SideEffectLevel,
    /// 稳定性级别
    #[serde(default)]
    pub stability: StabilityLevel,
    /// 扩展元数据
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
}

impl CapabilityDescriptor {
    /// 创建构建器，用于逐步构造能力描述符。
    pub fn builder(
        name: impl Into<String>,
        kind: impl Into<CapabilityKind>,
    ) -> CapabilityDescriptorBuilder {
        CapabilityDescriptorBuilder::new(name, kind)
    }

    /// 从能力名称中提取命名空间。
    ///
    /// 命名空间是名称中第一个点号之前的部分。
    /// 例如 `"tool.read_file"` → `"tool"`，`"shell"` → `"shell"`。
    ///
    /// 运行时和插件注册路径会调用此方法，确保插件作者即使不使用 Builder API
    /// 也能获得与 Builder 相同的校验保证。
    pub fn namespace(&self) -> &str {
        self.name
            .split_once('.')
            .map_or(self.name.as_str(), |(ns, _)| ns)
    }

    /// 验证描述符的完整性。
    ///
    /// 校验包括：必填字段非空、schema 为 JSON 对象、列表无重复值等。
    /// 运行时和插件注册路径会调用此方法。
    pub fn validate(&self) -> Result<(), DescriptorBuildError> {
        validate_non_empty("name", self.name.clone())?;
        validate_kind(self.kind.clone())?;
        validate_non_empty("description", self.description.clone())?;
        validate_schema("input_schema", self.input_schema.clone())?;
        validate_schema("output_schema", self.output_schema.clone())?;
        validate_string_list("profiles", self.profiles.clone())?;
        validate_string_list("tags", self.tags.clone())?;
        validate_permissions(self.permissions.clone())?;
        Ok(())
    }
}

/// 描述符构建期间的校验错误。
///
/// 包含字段为空、缺失、schema 格式错误、列表重复等错误类型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescriptorBuildError {
    /// 必填字段为空字符串
    EmptyField(&'static str),
    /// 必填字段缺失
    MissingField(&'static str),
    /// Schema 不是有效的 JSON 对象
    InvalidSchema(&'static str),
    /// 列表中存在重复值
    DuplicateValue { field: &'static str, value: String },
}

impl fmt::Display for DescriptorBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField(field) => write!(f, "descriptor field '{field}' cannot be empty"),
            Self::MissingField(field) => write!(f, "descriptor field '{field}' is required"),
            Self::InvalidSchema(field) => {
                write!(f, "descriptor field '{field}' must be a JSON object schema")
            },
            Self::DuplicateValue { field, value } => {
                write!(
                    f,
                    "descriptor field '{field}' contains duplicate value '{value}'"
                )
            },
        }
    }
}

impl std::error::Error for DescriptorBuildError {}

/// `CapabilityDescriptor` 的构建器。
///
/// 采用 builder pattern 逐步构造能力描述符，在 `build()` 时执行完整校验。
/// 相比直接构造 `CapabilityDescriptor`，builder 可以确保所有必填字段都已设置。
#[derive(Debug, Clone)]
pub struct CapabilityDescriptorBuilder {
    name: String,
    kind: CapabilityKind,
    description: Option<String>,
    input_schema: Option<Value>,
    output_schema: Option<Value>,
    streaming: bool,
    concurrency_safe: bool,
    compact_clearable: bool,
    profiles: Vec<String>,
    tags: Vec<String>,
    permissions: Vec<PermissionHint>,
    side_effect: SideEffectLevel,
    stability: StabilityLevel,
    metadata: Value,
}

impl CapabilityDescriptorBuilder {
    /// 创建新的构建器，仅需名称和能力类型。
    pub fn new(name: impl Into<String>, kind: impl Into<CapabilityKind>) -> Self {
        Self {
            name: name.into(),
            kind: kind.into(),
            description: None,
            input_schema: None,
            output_schema: None,
            streaming: false,
            concurrency_safe: false,
            compact_clearable: false,
            profiles: Vec::new(),
            tags: Vec::new(),
            permissions: Vec::new(),
            side_effect: SideEffectLevel::default(),
            stability: StabilityLevel::default(),
            metadata: Value::Null,
        }
    }

    /// 设置能力描述。
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// 设置输入 JSON Schema。
    pub fn input_schema(mut self, input_schema: Value) -> Self {
        self.input_schema = Some(input_schema);
        self
    }

    /// 设置输出 JSON Schema。
    pub fn output_schema(mut self, output_schema: Value) -> Self {
        self.output_schema = Some(output_schema);
        self
    }

    /// 同时设置输入和输出 JSON Schema。
    pub fn schema(mut self, input_schema: Value, output_schema: Value) -> Self {
        self.input_schema = Some(input_schema);
        self.output_schema = Some(output_schema);
        self
    }

    /// 设置是否支持流式输出。
    pub fn streaming(mut self, streaming: bool) -> Self {
        self.streaming = streaming;
        self
    }

    /// 设置是否可安全并发调用。
    pub fn concurrency_safe(mut self, concurrency_safe: bool) -> Self {
        self.concurrency_safe = concurrency_safe;
        self
    }

    /// 设置在上下文压缩时是否可被清除。
    pub fn compact_clearable(mut self, compact_clearable: bool) -> Self {
        self.compact_clearable = compact_clearable;
        self
    }

    /// 添加单个 profile。
    pub fn profile(mut self, profile: impl Into<String>) -> Self {
        self.profiles.push(profile.into());
        self
    }

    /// 批量添加 profiles。
    pub fn profiles<I, S>(mut self, profiles: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.profiles.extend(profiles.into_iter().map(Into::into));
        self
    }

    /// 添加单个标签。
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// 批量添加标签。
    pub fn tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.tags.extend(tags.into_iter().map(Into::into));
        self
    }

    /// 添加权限（不含理由）。
    pub fn permission(mut self, name: impl Into<String>) -> Self {
        self.permissions.push(PermissionHint {
            name: name.into(),
            rationale: None,
        });
        self
    }

    /// 添加权限并附带理由。
    pub fn permission_with_rationale(
        mut self,
        name: impl Into<String>,
        rationale: impl Into<String>,
    ) -> Self {
        self.permissions.push(PermissionHint {
            name: name.into(),
            rationale: Some(rationale.into()),
        });
        self
    }

    /// 批量添加权限。
    pub fn permissions(mut self, permissions: Vec<PermissionHint>) -> Self {
        self.permissions.extend(permissions);
        self
    }

    /// 设置副作用级别。
    pub fn side_effect(mut self, side_effect: SideEffectLevel) -> Self {
        self.side_effect = side_effect;
        self
    }

    /// 设置稳定性级别。
    pub fn stability(mut self, stability: StabilityLevel) -> Self {
        self.stability = stability;
        self
    }

    /// 设置扩展元数据。
    pub fn metadata(mut self, metadata: Value) -> Self {
        self.metadata = metadata;
        self
    }

    /// 构建并校验能力描述符。
    ///
    /// 校验所有必填字段和格式约束，失败时返回 `DescriptorBuildError`。
    pub fn build(self) -> Result<CapabilityDescriptor, DescriptorBuildError> {
        let name = validate_non_empty("name", self.name)?;
        let kind = validate_kind(self.kind)?;
        let description = validate_non_empty(
            "description",
            self.description
                .ok_or(DescriptorBuildError::MissingField("description"))?,
        )?;
        let input_schema = validate_schema(
            "input_schema",
            self.input_schema
                .ok_or(DescriptorBuildError::MissingField("input_schema"))?,
        )?;
        let output_schema = validate_schema(
            "output_schema",
            self.output_schema
                .ok_or(DescriptorBuildError::MissingField("output_schema"))?,
        )?;
        let profiles = validate_string_list("profiles", self.profiles)?;
        let tags = validate_string_list("tags", self.tags)?;
        let permissions = validate_permissions(self.permissions)?;

        Ok(CapabilityDescriptor {
            name,
            kind,
            description,
            input_schema,
            output_schema,
            streaming: self.streaming,
            concurrency_safe: self.concurrency_safe,
            compact_clearable: self.compact_clearable,
            profiles,
            tags,
            permissions,
            side_effect: self.side_effect,
            stability: self.stability,
            metadata: self.metadata,
        })
    }
}

/// serde 辅助函数：判断 bool 是否为 false，用于 `skip_serializing_if`。
fn is_false(value: &bool) -> bool {
    !*value
}

/// 校验字符串字段非空，返回 trim 后的副本。
fn validate_non_empty(field: &'static str, value: String) -> Result<String, DescriptorBuildError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(DescriptorBuildError::EmptyField(field));
    }
    Ok(trimmed.to_string())
}

/// 校验能力类型非空。
fn validate_kind(value: CapabilityKind) -> Result<CapabilityKind, DescriptorBuildError> {
    Ok(CapabilityKind(validate_non_empty("kind", value.0)?))
}

/// 校验 schema 为 JSON 对象类型。
fn validate_schema(field: &'static str, value: Value) -> Result<Value, DescriptorBuildError> {
    if value.is_object() {
        Ok(value)
    } else {
        Err(DescriptorBuildError::InvalidSchema(field))
    }
}

/// 校验字符串列表：每项非空且无重复。
///
/// 使用 `BTreeSet` 进行去重检测，保证确定性排序。
fn validate_string_list(
    field: &'static str,
    values: Vec<String>,
) -> Result<Vec<String>, DescriptorBuildError> {
    let mut seen = std::collections::BTreeSet::new();
    let mut normalized = Vec::with_capacity(values.len());
    for value in values {
        let value = validate_non_empty(field, value)?;
        if !seen.insert(value.clone()) {
            return Err(DescriptorBuildError::DuplicateValue { field, value });
        }
        normalized.push(value);
    }
    Ok(normalized)
}

/// 校验权限列表：名称非空且无重复。
fn validate_permissions(
    permissions: Vec<PermissionHint>,
) -> Result<Vec<PermissionHint>, DescriptorBuildError> {
    let mut seen = std::collections::BTreeSet::new();
    let mut normalized = Vec::with_capacity(permissions.len());
    for permission in permissions {
        let name = validate_non_empty("permissions", permission.name)?;
        if !seen.insert(name.clone()) {
            return Err(DescriptorBuildError::DuplicateValue {
                field: "permissions",
                value: name,
            });
        }
        normalized.push(PermissionHint {
            name,
            rationale: permission
                .rationale
                .map(|rationale| rationale.trim().to_string())
                .filter(|rationale| !rationale.is_empty()),
        });
    }
    Ok(normalized)
}

/// 触发器描述符。
///
/// 用于 `HandlerDescriptor` 中定义什么事件会触发此处理器。
/// `kind` 标识触发器类型（如 `file_change`, `session_start`），`value` 为匹配值。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TriggerDescriptor {
    /// 触发器类型
    pub kind: String,
    /// 匹配值
    pub value: String,
    /// 扩展元数据
    #[serde(default)]
    pub metadata: Value,
}

/// 过滤器描述符。
///
/// 用于 `HandlerDescriptor` 中对事件进行条件过滤。
/// `field` 为要检查的字段名，`op` 为操作符（如 `eq`, `contains`），`value` 为匹配值。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FilterDescriptor {
    /// 要过滤的字段名
    pub field: String,
    /// 过滤操作符
    pub op: String,
    /// 匹配值
    pub value: String,
}

/// 事件处理器描述符。
///
/// 描述一个可以响应特定事件的处理器的触发条件、输入 schema 和权限。
/// 与 `CapabilityDescriptor` 不同，handler 是被动触发而非主动调用。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HandlerDescriptor {
    /// 处理器唯一标识
    pub id: String,
    /// 触发条件
    pub trigger: TriggerDescriptor,
    /// 输入参数的 JSON Schema
    pub input_schema: Value,
    /// 此处理器可用的 profile 列表
    #[serde(default)]
    pub profiles: Vec<String>,
    /// 事件过滤规则列表（所有规则必须满足）
    #[serde(default)]
    pub filters: Vec<FilterDescriptor>,
    /// 需要的权限列表
    #[serde(default)]
    pub permissions: Vec<PermissionHint>,
}

/// Profile 描述符。
///
/// 描述一个能力 profile 的元数据，包括版本、上下文 schema 等。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProfileDescriptor {
    /// profile 名称
    pub name: String,
    /// profile 版本
    pub version: String,
    /// profile 描述
    pub description: String,
    /// 上下文数据的 JSON Schema
    #[serde(default)]
    pub context_schema: Value,
    /// 扩展元数据
    #[serde(default)]
    pub metadata: Value,
}

/// 调用方引用。
///
/// 在 `InvocationContext` 中标识发起调用的对等方。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CallerRef {
    /// 调用方 ID
    pub id: String,
    /// 调用方角色
    pub role: String,
    /// 扩展元数据
    #[serde(default)]
    pub metadata: Value,
}

/// 工作区引用。
///
/// 在 `InvocationContext` 中提供工作区上下文信息。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRef {
    /// 当前工作目录
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// Git 仓库根目录
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_root: Option<String>,
    /// 当前 Git 分支
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// 扩展元数据
    #[serde(default)]
    pub metadata: Value,
}

/// 预算提示。
///
/// 在 `InvocationContext` 中为插件提供资源限制建议。
/// 这些是提示值而非硬性限制，插件应尽力遵守。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BudgetHint {
    /// 建议的最大执行时间（毫秒）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_duration_ms: Option<u64>,
    /// 建议的最大事件数量
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_events: Option<u64>,
    /// 建议的最大输出字节数
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<u64>,
}

/// 调用上下文。
///
/// 每次 `InvokeMessage` 都携带此上下文，为插件提供调用方身份、工作区信息、
/// 预算限制等元数据，使插件可以做出更智能的决策。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InvocationContext {
    /// 请求唯一标识
    pub request_id: String,
    /// 分布式追踪 ID（可选）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    /// 关联的会话 ID
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// 调用方信息
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller: Option<CallerRef>,
    /// 工作区上下文
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceRef>,
    /// 超时时间（毫秒），从请求发出开始计算
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<u64>,
    /// 资源预算建议
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<BudgetHint>,
    /// 当前活跃的 profile 名称
    pub profile: String,
    /// profile 相关的上下文数据
    #[serde(default)]
    pub profile_context: Value,
    /// 扩展元数据
    #[serde(default)]
    pub metadata: Value,
}
