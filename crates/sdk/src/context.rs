//! 插件上下文数据模型。
//!
//! 本模块定义了插件在执行工具或钩子时可访问的上下文信息，
//! 包括工作区状态、编码配置文件（coding profile）以及请求追踪元数据。
//!
//! ## 设计意图
//!
//! 插件不应直接访问宿主运行时内部状态，而是通过 `PluginContext` 获取
//! 与当前调用相关的快照。这保证了插件的可测试性和隔离性。
//!
//! ## 上下文层次
//!
//! - **WorkspaceContext**: 文件系统级别的工作区信息（工作目录、仓库根目录、分支）
//! - **CodingProfileContext**: 编辑器状态（打开的文件、选中区域、审批模式）
//! - **PluginContext**: 顶层上下文，聚合上述信息并附加请求/会话/追踪 ID

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 工作区级别的上下文信息。
///
/// 提供插件执行时所需的文件系统环境快照，
/// 使工具能够正确解析相对路径或感知当前 Git 状态。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceContext {
    /// 当前工作目录的绝对路径。
    ///
    /// 工具应以此为基础解析相对路径，而非依赖进程 CWD。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// Git 仓库根目录的绝对路径。
    ///
    /// 当工作区不在 Git 仓库中时为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_root: Option<String>,
    /// 当前 Git 分支名称。
    ///
    /// 非 Git 仓库或 detached HEAD 状态下为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

/// 编辑器中的文本选区。
///
/// 用于标识用户在编辑器中选中了哪段代码，
/// 行号从 1 开始，列号从 1 开始（如果提供）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextSelection {
    /// 选区起始行号（1-based）。
    pub start_line: u64,
    /// 选区起始列号（1-based），未指定时表示整行。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_column: Option<u64>,
    /// 选区结束行号（1-based）。
    pub end_line: u64,
    /// 选区结束列号（1-based），未指定时表示整行。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_column: Option<u64>,
}

/// 编码配置文件（coding profile）的上下文信息。
///
/// 当插件在 "coding" 模式下被调用时，此结构体包含编辑器当前的状态快照，
/// 使工具能够感知用户正在查看的文件、选中的代码段等上下文。
///
/// ## 为什么使用 `extras: Value`
///
/// 编码配置文件可能随版本演进增加新字段，`extras` 作为扩展点
/// 容纳未显式建模的字段，避免反序列化失败。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingProfileContext {
    /// 当前工作目录的绝对路径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    /// Git 仓库根目录的绝对路径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_root: Option<String>,
    /// 当前在编辑器中打开的文件路径列表。
    #[serde(default)]
    pub open_files: Vec<String>,
    /// 当前活动（焦点所在）的文件路径。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_file: Option<String>,
    /// 用户在活动文件中的文本选区（如果有）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<TextSelection>,
    /// 当前的审批模式（如 "auto", "approve-each" 等）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_mode: Option<String>,
    /// 扩展字段，容纳未来可能新增的配置文件属性。
    #[serde(default)]
    pub extras: Value,
}

/// 插件调用上下文。
///
/// 这是插件在执行工具或钩子时接收到的完整上下文快照，
/// 包含请求追踪信息、工作区状态和当前配置文件。
///
/// ## 生命周期
///
/// 每次工具调用都会创建一个新的 `PluginContext` 实例，
/// 插件不应跨调用缓存此结构体，因为其中的状态可能已过期。
///
/// ## 从协议类型转换
///
/// 实现了 `From<InvocationContext>`，由运行时在调用插件前自动转换，
/// 插件作者无需关心协议层细节。
#[derive(Debug, Clone, PartialEq)]
pub struct PluginContext {
    /// 本次请求的唯一标识符。
    ///
    /// 用于日志关联和调试，贯穿整个请求链路。
    pub request_id: String,
    /// 会话 ID（如果存在）。
    ///
    /// 标识本次调用所属的对话会话，可用于会话级别的缓存或状态管理。
    pub session_id: Option<String>,
    /// 分布式追踪 ID（如果存在）。
    ///
    /// 用于跨服务追踪，将插件调用与 LLM 请求、工具执行等关联起来。
    pub trace_id: Option<String>,
    /// 工作区上下文快照。
    ///
    /// 当调用不涉及工作区时（如纯计算工具）可能为 `None`。
    pub workspace: Option<WorkspaceContext>,
    /// 当前配置文件名称（如 "coding"、"planning" 等）。
    ///
    /// 插件可据此调整行为，例如在 "coding" 模式下感知编辑器状态。
    pub profile: String,
    /// 配置文件的原始 JSON 上下文。
    ///
    /// 通过 `coding_profile()` 方法可尝试解析为 `CodingProfileContext`。
    pub profile_context: Value,
}

impl Default for PluginContext {
    /// 创建空的插件上下文。
    ///
    /// 主要用于测试和默认值场景，实际调用中上下文由运行时注入。
    fn default() -> Self {
        Self {
            request_id: String::new(),
            session_id: None,
            trace_id: None,
            workspace: None,
            profile: "coding".to_string(),
            profile_context: Value::Null,
        }
    }
}

impl PluginContext {
    /// 尝试将配置文件上下文解析为 `CodingProfileContext`。
    ///
    /// 仅当 `profile` 字段为 `"coding"` 时返回 `Some`，
    /// 其他配置文件模式（如 "planning"）返回 `None`。
    ///
    /// ## 为什么返回 Option 而不是 Result
    ///
    /// 配置文件不匹配不是错误，而是正常的业务分支。
    /// 反序列化失败也被静默吞掉，因为 `extras` 字段已容纳未知数据，
    /// 真正的结构不匹配意味着协议版本不一致，应由上层处理。
    pub fn coding_profile(&self) -> Option<CodingProfileContext> {
        if self.profile != "coding" {
            return None;
        }
        serde_json::from_value(self.profile_context.clone()).ok()
    }
}

impl From<astrcode_protocol::plugin::InvocationContext> for PluginContext {
    fn from(value: astrcode_protocol::plugin::InvocationContext) -> Self {
        Self {
            request_id: value.request_id,
            session_id: value.session_id,
            trace_id: value.trace_id,
            workspace: value.workspace.map(|workspace| WorkspaceContext {
                working_dir: workspace.working_dir,
                repo_root: workspace.repo_root,
                branch: workspace.branch,
            }),
            profile: value.profile,
            profile_context: value.profile_context,
        }
    }
}
