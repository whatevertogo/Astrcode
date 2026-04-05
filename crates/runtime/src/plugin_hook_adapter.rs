//! # Plugin Hook Adapter
//!
//! 将插件协议里的 `HandlerDescriptor` 适配为 runtime 内部的 `HookHandler`。
//! 当前约定：
//! - `trigger.kind` 必须为 `"lifecycle"`
//! - `trigger.value` 映射到内部 `HookEvent`
//! - `handler.id` 直接作为远端可调用 handler 名称
//!
//! 这里暂时不再引入新的协议层方法，优先复用现有的 `InvokeMessage` 通道。
//!
//! TODO(hook-protocol): 现阶段插件 hook 仍复用普通 capability invoke 通道，
//! 后续应在 protocol/plugin 层补上正式的 hook 调用协议和稳定 schema，
//! 避免 `handler.id` / `{ action, args, reason }` 这类约定散落在宿主与插件实现中。

use std::sync::Arc;

use astrcode_core::{
    AstrError, CompactionHookContext, CompactionHookResultContext, HookEvent, HookHandler,
    HookInput, HookOutcome, Result, ToolHookContext, ToolHookResultContext,
};
use astrcode_plugin::Supervisor;
use astrcode_protocol::plugin::{
    FilterDescriptor, HandlerDescriptor, InvocationContext, WorkspaceRef,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct PluginHookHandler {
    plugin_name: String,
    remote_handler_id: String,
    event: HookEvent,
    filters: Vec<FilterDescriptor>,
    supervisor: Arc<Supervisor>,
}

impl PluginHookHandler {
    fn new(
        plugin_name: String,
        remote_handler_id: String,
        event: HookEvent,
        filters: Vec<FilterDescriptor>,
        supervisor: Arc<Supervisor>,
    ) -> Self {
        Self {
            plugin_name,
            remote_handler_id,
            event,
            filters,
            supervisor,
        }
    }
}

#[async_trait]
impl HookHandler for PluginHookHandler {
    fn name(&self) -> &str {
        &self.remote_handler_id
    }

    fn event(&self) -> HookEvent {
        self.event
    }

    fn matches(&self, input: &HookInput) -> bool {
        self.filters
            .iter()
            .all(|filter| filter_matches(filter, input))
    }

    async fn run(&self, input: &HookInput) -> Result<HookOutcome> {
        let result = self
            .supervisor
            .invoke(
                self.remote_handler_id.clone(),
                hook_input_payload(input)?,
                invocation_context_for_hook(input),
            )
            .await?;

        if !result.success {
            let error = result
                .error
                .map(|payload| payload.message)
                .unwrap_or_else(|| "plugin hook invocation failed".to_string());
            return Err(AstrError::ToolError {
                name: format!(
                    "plugin hook {}:{}",
                    self.plugin_name, self.remote_handler_id
                ),
                reason: error,
            });
        }

        parse_plugin_hook_outcome(result.output)
    }
}

/// 插件 hook 响应格式。
///
/// 支持以下 action：
/// - `continue`: 不做修改，继续执行
/// - `block`: 阻止操作，需要提供 reason
/// - `replaceToolArgs`: 替换工具参数（仅 PreToolUse）
/// - `modifyCompactContext`: 修改压缩上下文（仅 PreCompact）
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PluginHookResponse {
    action: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    args: Option<Value>,
    /// PreCompact 专用：追加到默认 compact prompt 后的 system prompt 片段
    #[serde(default)]
    additional_system_prompt: Option<String>,
    /// PreCompact 专用：覆盖保留的最近 turn 数量
    #[serde(default)]
    override_keep_recent_turns: Option<usize>,
    /// PreCompact 专用：自定义摘要内容（跳过 LLM 调用）
    #[serde(default)]
    custom_summary: Option<String>,
}

pub(crate) fn build_plugin_hook_handlers(
    plugin_name: &str,
    handlers: &[HandlerDescriptor],
    supervisor: Arc<Supervisor>,
) -> Vec<Arc<dyn HookHandler>> {
    // TODO(hook-capabilities): 当前只消费 `handlers`，尚未把 permissions / input_schema
    // 接进宿主侧校验与审批链路。等 hook 协议稳定后，这里应统一走和 capability
    // 更接近的治理模型，而不是仅做轻量触发映射。
    handlers
        .iter()
        .filter_map(|handler| {
            let Some(event) = hook_event_from_handler(handler) else {
                log::warn!(
                    "plugin '{}' handler '{}' ignored: unsupported lifecycle trigger {}:{}",
                    plugin_name,
                    handler.id,
                    handler.trigger.kind,
                    handler.trigger.value
                );
                return None;
            };

            Some(Arc::new(PluginHookHandler::new(
                plugin_name.to_string(),
                handler.id.clone(),
                event,
                handler.filters.clone(),
                Arc::clone(&supervisor),
            )) as Arc<dyn HookHandler>)
        })
        .collect()
}

fn hook_event_from_handler(handler: &HandlerDescriptor) -> Option<HookEvent> {
    if handler.trigger.kind != "lifecycle" {
        return None;
    }

    match handler.trigger.value.as_str() {
        "pre_tool_use" => Some(HookEvent::PreToolUse),
        "post_tool_use" => Some(HookEvent::PostToolUse),
        "post_tool_use_failure" => Some(HookEvent::PostToolUseFailure),
        "pre_compact" => Some(HookEvent::PreCompact),
        "post_compact" => Some(HookEvent::PostCompact),
        _ => None,
    }
}

fn filter_matches(filter: &FilterDescriptor, input: &HookInput) -> bool {
    let Some(actual) = hook_input_field(input, &filter.field) else {
        return false;
    };

    match filter.op.as_str() {
        "eq" => actual == filter.value,
        "contains" => actual.contains(&filter.value),
        unsupported => {
            log::warn!(
                "unsupported plugin hook filter op '{}' on field '{}', treating as non-match",
                unsupported,
                filter.field
            );
            false
        },
    }
}

fn hook_input_field(input: &HookInput, field: &str) -> Option<String> {
    match input {
        HookInput::PreToolUse(tool) => tool_field(tool, field),
        HookInput::PostToolUse(tool) | HookInput::PostToolUseFailure(tool) => {
            tool_result_field(tool, field)
        },
        HookInput::PreCompact(compaction) => compaction_field(compaction, field),
        HookInput::PostCompact(compaction) => compaction_result_field(compaction, field),
    }
}

fn tool_field(tool: &ToolHookContext, field: &str) -> Option<String> {
    match field {
        "event" => Some("pre_tool_use".to_string()),
        "sessionId" => Some(tool.session_id.clone()),
        "turnId" => Some(tool.turn_id.clone()),
        "toolName" => Some(tool.tool_name.clone()),
        "toolCallId" => Some(tool.tool_call_id.clone()),
        "workingDir" => Some(tool.working_dir.display().to_string()),
        _ => None,
    }
}

fn tool_result_field(tool: &ToolHookResultContext, field: &str) -> Option<String> {
    match field {
        "event" => Some(if tool.result.ok {
            "post_tool_use".to_string()
        } else {
            "post_tool_use_failure".to_string()
        }),
        "ok" => Some(tool.result.ok.to_string()),
        "error" => tool.result.error.clone(),
        "output" => Some(tool.result.output.clone()),
        _ => tool_field(&tool.tool, field),
    }
}

fn compaction_field(compaction: &CompactionHookContext, field: &str) -> Option<String> {
    match field {
        "event" => Some("pre_compact".to_string()),
        "sessionId" => Some(compaction.session_id.clone()),
        "workingDir" => Some(compaction.working_dir.display().to_string()),
        "reason" => Some(format!("{:?}", compaction.reason).to_lowercase()),
        _ => None,
    }
}

fn compaction_result_field(
    compaction: &CompactionHookResultContext,
    field: &str,
) -> Option<String> {
    match field {
        "event" => Some("post_compact".to_string()),
        "summary" => Some(compaction.summary.clone()),
        "strategyId" => Some(compaction.strategy_id.clone()),
        _ => compaction_field(&compaction.compaction, field),
    }
}

fn hook_input_payload(input: &HookInput) -> Result<Value> {
    serde_json::to_value(input).map_err(|error| {
        AstrError::Validation(format!(
            "failed to serialize hook input for plugin dispatch: {error}"
        ))
    })
}

fn invocation_context_for_hook(input: &HookInput) -> InvocationContext {
    let (session_id, working_dir) = match input {
        HookInput::PreToolUse(tool) => (&tool.session_id, &tool.working_dir),
        HookInput::PostToolUse(tool) | HookInput::PostToolUseFailure(tool) => {
            (&tool.tool.session_id, &tool.tool.working_dir)
        },
        HookInput::PreCompact(compaction) => (&compaction.session_id, &compaction.working_dir),
        HookInput::PostCompact(compaction) => (
            &compaction.compaction.session_id,
            &compaction.compaction.working_dir,
        ),
    };

    InvocationContext {
        request_id: format!("hook-{}", Uuid::new_v4()),
        trace_id: None,
        session_id: Some(session_id.clone()),
        caller: None,
        workspace: Some(WorkspaceRef {
            working_dir: Some(working_dir.display().to_string()),
            repo_root: Some(working_dir.display().to_string()),
            branch: None,
            metadata: Value::Null,
        }),
        deadline_ms: None,
        budget: None,
        profile: "coding".to_string(),
        profile_context: Value::Null,
        metadata: json!({
            "source": "runtime-hook-dispatch",
            "hookEvent": hook_event_name(input.event()),
        }),
    }
}

fn hook_event_name(event: HookEvent) -> &'static str {
    // TODO(hook-event-names): 这里的字符串需要和插件文档保持同步。
    // 后续如果 protocol 增加专门的 HookEvent DTO，应改成由共享枚举序列化，
    // 而不是继续手写映射。
    match event {
        HookEvent::PreToolUse => "pre_tool_use",
        HookEvent::PostToolUse => "post_tool_use",
        HookEvent::PostToolUseFailure => "post_tool_use_failure",
        HookEvent::PreCompact => "pre_compact",
        HookEvent::PostCompact => "post_compact",
    }
}

fn parse_plugin_hook_outcome(output: Value) -> Result<HookOutcome> {
    if output.is_null() {
        return Ok(HookOutcome::Continue);
    }

    let response: PluginHookResponse = serde_json::from_value(output).map_err(|error| {
        AstrError::Validation(format!(
            "plugin hook returned invalid outcome payload; expected {{action,...}}: {error}"
        ))
    })?;

    match response.action.as_str() {
        "continue" => Ok(HookOutcome::Continue),
        "block" => Ok(HookOutcome::Block {
            reason: response
                .reason
                .unwrap_or_else(|| "plugin hook blocked without reason".to_string()),
        }),
        "replaceToolArgs" => Ok(HookOutcome::ReplaceToolArgs {
            args: response.args.unwrap_or(Value::Null),
        }),
        "modifyCompactContext" => Ok(HookOutcome::ModifyCompactContext {
            additional_system_prompt: response.additional_system_prompt,
            override_keep_recent_turns: response.override_keep_recent_turns,
            custom_summary: response.custom_summary,
        }),
        other => Err(AstrError::Validation(format!(
            "plugin hook returned unsupported action '{}'",
            other
        ))),
    }
}

#[cfg(test)]
mod tests {
    use astrcode_core::{HookCompactionReason, HookInput};
    use astrcode_protocol::plugin::{FilterDescriptor, HandlerDescriptor, TriggerDescriptor};

    use super::*;

    #[test]
    fn maps_lifecycle_handler_to_hook_event() {
        let handler = HandlerDescriptor {
            id: "pre-tool".to_string(),
            trigger: TriggerDescriptor {
                kind: "lifecycle".to_string(),
                value: "pre_tool_use".to_string(),
                metadata: Value::Null,
            },
            input_schema: Value::Null,
            profiles: vec!["coding".to_string()],
            filters: Vec::new(),
            permissions: Vec::new(),
        };

        assert_eq!(
            hook_event_from_handler(&handler),
            Some(HookEvent::PreToolUse)
        );
    }

    #[test]
    fn filter_matching_supports_eq_and_contains() {
        let input = HookInput::PreCompact(CompactionHookContext {
            session_id: "session-1".to_string(),
            working_dir: std::path::PathBuf::from("D:/repo"),
            reason: HookCompactionReason::Manual,
            keep_recent_turns: 2,
            message_count: 4,
            messages: Vec::new(),
            tools: Vec::new(),
            system_prompt: None,
        });

        assert!(filter_matches(
            &FilterDescriptor {
                field: "reason".to_string(),
                op: "eq".to_string(),
                value: "manual".to_string(),
            },
            &input,
        ));
        assert!(filter_matches(
            &FilterDescriptor {
                field: "workingDir".to_string(),
                op: "contains".to_string(),
                value: "repo".to_string(),
            },
            &input,
        ));
    }

    #[test]
    fn plugin_hook_response_parses_supported_actions() {
        assert_eq!(
            parse_plugin_hook_outcome(json!({"action":"continue"})).expect("continue"),
            HookOutcome::Continue
        );
        assert_eq!(
            parse_plugin_hook_outcome(json!({"action":"block","reason":"stop"})).expect("block"),
            HookOutcome::Block {
                reason: "stop".to_string()
            }
        );
        assert_eq!(
            parse_plugin_hook_outcome(json!({"action":"replaceToolArgs","args":{"x":1}}))
                .expect("replace"),
            HookOutcome::ReplaceToolArgs {
                args: json!({"x":1})
            }
        );
    }

    #[test]
    fn plugin_hook_response_parses_modify_compact_context() {
        // 完整的 modifyCompactContext 响应
        let outcome = parse_plugin_hook_outcome(json!({
            "action": "modifyCompactContext",
            "additionalSystemPrompt": "Custom prompt",
            "overrideKeepRecentTurns": 5,
            "customSummary": "Custom summary content"
        }))
        .expect("modify compact context");

        match outcome {
            HookOutcome::ModifyCompactContext {
                additional_system_prompt,
                override_keep_recent_turns,
                custom_summary,
            } => {
                assert_eq!(additional_system_prompt, Some("Custom prompt".to_string()));
                assert_eq!(override_keep_recent_turns, Some(5));
                assert_eq!(custom_summary, Some("Custom summary content".to_string()));
            },
            _ => panic!("expected ModifyCompactContext"),
        }

        // 部分字段的 modifyCompactContext 响应
        let outcome = parse_plugin_hook_outcome(json!({
            "action": "modifyCompactContext",
            "overrideKeepRecentTurns": 3
        }))
        .expect("partial modify");

        match outcome {
            HookOutcome::ModifyCompactContext {
                additional_system_prompt,
                override_keep_recent_turns,
                custom_summary,
            } => {
                assert_eq!(additional_system_prompt, None);
                assert_eq!(override_keep_recent_turns, Some(3));
                assert_eq!(custom_summary, None);
            },
            _ => panic!("expected ModifyCompactContext"),
        }
    }
}
