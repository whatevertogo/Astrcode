//! # Tool Trait 与执行上下文
//!
//! 定义了工具（Tool）系统的核心抽象。Tool 是 LLM Agent 调用外部能力的统一接口。
//!
//! ## 核心概念
//!
//! - **Tool**: 可被 Agent 调用的能力单元（如文件读写、Shell 执行、代码搜索）
//! - **ToolContext**: 工具执行时的上下文信息（会话 ID、工作目录、取消令牌）
//! - **ToolCapabilityMetadata**: 工具的能力元数据（用于策略引擎的权限判断）

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::{
    CancelToken, CapabilityDescriptor, CapabilityKind, DescriptorBuildError, PermissionHint,
    Result, SideEffectLevel, StabilityLevel, ToolDefinition, ToolExecutionResult,
};

/// 会话 ID 类型别名
pub type SessionId = String;

/// 工具执行的默认最大输出大小（1 MB）
///
/// 超过此大小的输出会被截断，防止大文件导致内存溢出或网络传输问题。
pub const DEFAULT_MAX_OUTPUT_SIZE: usize = 1024 * 1024;

/// 工具执行上下文
///
/// 每次工具调用都会携带此上下文，包含：
/// - `session_id`: 标识所属会话
/// - `working_dir`: 工作目录（所有文件操作应基于此目录进行）
/// - `cancel`: 取消令牌（支持异步取消）
/// - `max_output_size`: 输出大小限制（防止过大的返回值）
#[derive(Clone, Debug)]
pub struct ToolContext {
    /// 所属会话的 ID
    pub session_id: SessionId,
    /// 工作目录（工具必须在此目录内操作，禁止访问路径外的文件）
    pub working_dir: PathBuf,
    /// 取消令牌（用于响应用户中断）
    pub cancel: CancelToken,
    /// 最大输出大小（字节），默认 1MB
    pub max_output_size: usize,
}

impl ToolContext {
    /// 创建新的工具执行上下文
    pub fn new(session_id: SessionId, working_dir: PathBuf, cancel: CancelToken) -> Self {
        Self {
            session_id,
            working_dir,
            cancel,
            max_output_size: DEFAULT_MAX_OUTPUT_SIZE,
        }
    }

    /// 设置自定义的最大输出大小
    pub fn with_max_output_size(mut self, max_output_size: usize) -> Self {
        self.max_output_size = max_output_size;
        self
    }
}

/// 工具的能力元数据
///
/// 这些元数据会被策略引擎使用，决定：
/// - 该工具是否需要用户审批
/// - 工具属于哪些 Profile（如 "coding"、"browsing"）
/// - 工具的副作用级别（是否修改文件系统）
/// - 工具的稳定性级别（Experimental / Stable）
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolCapabilityMetadata {
    /// 该工具所属的 Profile 列表（用于权限分组）
    pub profiles: Vec<String>,
    /// 标签列表（用于 UI 分类和搜索）
    pub tags: Vec<String>,
    /// 所需权限提示（如 "filesystem.read"）
    pub permissions: Vec<PermissionHint>,
    /// 副作用级别（决定是否需要审批）
    pub side_effect: SideEffectLevel,
    /// 稳定性级别（实验性 API 可能破坏变更）
    pub stability: StabilityLevel,
}

impl Default for ToolCapabilityMetadata {
    fn default() -> Self {
        Self::builtin()
    }
}

impl ToolCapabilityMetadata {
    /// 创建内置工具的默认元数据
    ///
    /// 内置工具默认属于 "coding" profile，带 "builtin" 标签，
    /// 有 Workspace 级别的副作用，且为 Stable 稳定性。
    pub fn builtin() -> Self {
        Self {
            profiles: vec!["coding".to_string()],
            tags: vec!["builtin".to_string()],
            permissions: Vec::new(),
            side_effect: SideEffectLevel::Workspace,
            stability: StabilityLevel::Stable,
        }
    }

    /// 添加一个 Profile
    pub fn profile(mut self, profile: impl Into<String>) -> Self {
        self.profiles.push(profile.into());
        self
    }

    /// 批量添加多个 Profile
    pub fn profiles<I, S>(mut self, profiles: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.profiles.extend(profiles.into_iter().map(Into::into));
        self
    }

    /// 添加一个标签
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// 批量添加多个标签
    pub fn tags<I, S>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.tags.extend(tags.into_iter().map(Into::into));
        self
    }

    /// 添加一个权限提示（不需要理由）
    pub fn permission(mut self, name: impl Into<String>) -> Self {
        self.permissions.push(PermissionHint {
            name: name.into(),
            rationale: None,
        });
        self
    }

    /// 添加一个权限提示（带理由）
    ///
    /// 理由会被展示给用户，说明为什么需要这个权限。
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

    /// 设置副作用级别
    pub fn side_effect(mut self, side_effect: SideEffectLevel) -> Self {
        self.side_effect = side_effect;
        self
    }

    /// 设置稳定性级别
    pub fn stability(mut self, stability: StabilityLevel) -> Self {
        self.stability = stability;
        self
    }

    /// 将元数据构建为完整的能力描述符
    ///
    /// 此方法将工具定义与能力元数据结合，生成策略引擎使用的完整描述符。
    pub fn build_descriptor(
        self,
        definition: ToolDefinition,
    ) -> std::result::Result<CapabilityDescriptor, DescriptorBuildError> {
        CapabilityDescriptor::builder(definition.name, CapabilityKind::tool())
            .description(definition.description)
            .schema(definition.parameters, json!({ "type": "string" }))
            .profiles(self.profiles)
            .tags(self.tags)
            .permissions(self.permissions)
            .side_effect(self.side_effect)
            .stability(self.stability)
            .build()
    }
}

/// Tool trait：所有可被 Agent 调用的工具必须实现此接口
///
/// ## 生命周期
///
/// 1. Agent 决定调用工具
/// 2. 策略引擎检查是否需要审批（使用 `capability_descriptor`）
/// 3. 用户审批通过（如需要）
/// 4. 调用 `execute` 方法执行工具
/// 5. 返回 `ToolExecutionResult`
///
/// ## 线程安全
///
/// Tool 必须是 `Send + Sync`，因为同一个 Tool 实例可能被多个并发调用使用。
#[async_trait]
pub trait Tool: Send + Sync {
    /// 返回工具的定义（名称、描述、参数 schema）
    fn definition(&self) -> ToolDefinition;

    /// 返回工具的能力元数据
    ///
    /// 此方法允许工具实现声明自己的策略相关元数据，而不是在适配器中硬编码。
    /// 大多数工具只需重写此方法；高级工具可以重写 `capability_descriptor`。
    fn capability_metadata(&self) -> ToolCapabilityMetadata {
        ToolCapabilityMetadata::builtin()
    }

    /// 返回工具的完整能力描述符
    ///
    /// 默认实现将 `definition` 和 `capability_metadata` 结合构建描述符。
    /// 高级工具可以完全重写此方法以提供自定义描述符。
    fn capability_descriptor(
        &self,
    ) -> std::result::Result<CapabilityDescriptor, DescriptorBuildError> {
        self.capability_metadata()
            .build_descriptor(self.definition())
    }

    /// 执行工具调用
    ///
    /// ## 参数
    ///
    /// - `tool_call_id`: 此次调用的唯一标识符
    /// - `input`: 工具的输入参数（已解析为 JSON）
    /// - `ctx`: 执行上下文（包含会话信息、工作目录、取消令牌等）
    ///
    /// ## 返回
    ///
    /// 返回 `ToolExecutionResult`，包含：
    /// - `ok`: 操作是否成功
    /// - `output`: 工具输出
    /// - `error`: 错误信息（如失败）
    /// - `metadata`: 额外的元数据（如截断标记）
    ///
    /// ## 错误处理
    ///
    /// - 返回 `Err` 表示系统级错误（IO 错误、参数解析失败、用户取消）
    /// - 返回 `ok: false` 表示工具级拒绝（策略拒绝、参数验证失败）
    async fn execute(
        &self,
        tool_call_id: String,
        input: Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult>;
}
