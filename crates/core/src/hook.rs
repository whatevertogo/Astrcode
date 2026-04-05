//! # Lifecycle Hook 契约
//!
//! 将“可拦截的生命周期点”和“纯事件广播”分开，避免再引入第二条事实来源。
//! Hook 只负责少数明确的执行节点，且输入输出必须是强类型的。

use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Result, ToolExecutionResult};

/// 可被外部扩展拦截的生命周期事件。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    PostToolUseFailure,
    PreCompact,
    PostCompact,
}

/// Hook 视角下的压缩触发原因。
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum HookCompactionReason {
    Auto,
    Reactive,
    Manual,
}

/// 工具调用的公共上下文。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolHookContext {
    pub session_id: String,
    pub turn_id: String,
    pub working_dir: PathBuf,
    pub tool_call_id: String,
    pub tool_name: String,
    pub args: Value,
}

/// 工具调用完成后的上下文。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolHookResultContext {
    pub tool: ToolHookContext,
    pub result: ToolExecutionResult,
}

/// 压缩前公共上下文。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompactionHookContext {
    pub session_id: String,
    pub working_dir: PathBuf,
    pub reason: HookCompactionReason,
    pub keep_recent_turns: usize,
    pub message_count: usize,
}

/// 压缩完成后的上下文。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CompactionHookResultContext {
    pub compaction: CompactionHookContext,
    pub summary: String,
    pub strategy_id: String,
    pub preserved_recent_turns: usize,
    pub pre_tokens: usize,
    pub post_tokens_estimate: usize,
    pub messages_removed: usize,
    pub tokens_freed: usize,
}

/// Hook 统一输入。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum HookInput {
    PreToolUse(ToolHookContext),
    PostToolUse(ToolHookResultContext),
    PostToolUseFailure(ToolHookResultContext),
    PreCompact(CompactionHookContext),
    PostCompact(CompactionHookResultContext),
}

impl HookInput {
    pub fn event(&self) -> HookEvent {
        match self {
            Self::PreToolUse(_) => HookEvent::PreToolUse,
            Self::PostToolUse(_) => HookEvent::PostToolUse,
            Self::PostToolUseFailure(_) => HookEvent::PostToolUseFailure,
            Self::PreCompact(_) => HookEvent::PreCompact,
            Self::PostCompact(_) => HookEvent::PostCompact,
        }
    }
}

/// Hook 对生命周期流程施加的影响。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum HookOutcome {
    /// 不改变当前流程，继续执行下一个 hook。
    Continue,
    /// 明确阻止当前动作继续。
    Block { reason: String },
    /// 仅允许 `PreToolUse` 修改工具参数。
    ReplaceToolArgs { args: Value },
}

#[async_trait]
pub trait HookHandler: Send + Sync {
    /// 稳定的人类可读名称，用于日志和报错。
    fn name(&self) -> &str;

    /// 声明本 handler 关注的生命周期节点。
    fn event(&self) -> HookEvent;

    /// 可选匹配器，用于做 tool name / source 等更细粒度筛选。
    fn matches(&self, input: &HookInput) -> bool {
        let _ = input;
        true
    }

    /// 执行 hook。
    async fn run(&self, input: &HookInput) -> Result<HookOutcome>;
}
