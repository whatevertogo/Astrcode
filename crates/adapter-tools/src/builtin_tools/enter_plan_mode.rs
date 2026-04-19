//! `enterPlanMode` 工具。
//!
//! 允许模型在执行 mode 中显式切换到 plan mode，把“先规划再执行”做成正式状态迁移，
//! 而不是只靠提示词暗示。

use std::time::Instant;

use astrcode_core::{
    AstrError, ModeId, Result, SideEffect, Tool, ToolCapabilityMetadata, ToolContext,
    ToolDefinition, ToolExecutionResult, ToolPromptMetadata,
};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::builtin_tools::mode_transition::emit_mode_changed;

#[derive(Default)]
pub struct EnterPlanModeTool;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnterPlanModeArgs {
    #[serde(default)]
    reason: Option<String>,
}

#[async_trait]
impl Tool for EnterPlanModeTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "enterPlanMode".to_string(),
            description: "Switch the current session into plan mode before doing planning-heavy \
                          work."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "reason": {
                        "type": "string",
                        "description": "Optional short reason for switching into plan mode."
                    }
                },
                "additionalProperties": false
            }),
        }
    }

    fn capability_metadata(&self) -> ToolCapabilityMetadata {
        ToolCapabilityMetadata::builtin()
            .tags(["plan", "mode", "session"])
            .side_effect(SideEffect::Local)
            .prompt(
                ToolPromptMetadata::new(
                    "Switch the current session into plan mode.",
                    "Use `enterPlanMode` when the task needs an explicit planning phase before \
                     execution, or when the user directly asks for a plan. After entering, \
                     inspect the relevant code and tests, keep updating the session plan artifact \
                     until it is executable, then use `exitPlanMode` to present the finalized \
                     plan.",
                )
                .example(
                    "{ reason: \"Need to inspect the codebase and propose a safe refactor plan\" }",
                )
                .prompt_tag("plan"),
            )
    }

    async fn execute(
        &self,
        tool_call_id: String,
        args: Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        let started_at = Instant::now();
        let args: EnterPlanModeArgs = serde_json::from_value(args)
            .map_err(|error| AstrError::parse("invalid args for enterPlanMode", error))?;
        let from_mode_id = ctx.current_mode_id().clone();
        let to_mode_id = ModeId::plan();

        if from_mode_id == to_mode_id {
            return Ok(ToolExecutionResult {
                tool_call_id,
                tool_name: "enterPlanMode".to_string(),
                ok: true,
                output: "session is already in plan mode".to_string(),
                error: None,
                metadata: Some(mode_metadata(
                    &from_mode_id,
                    &to_mode_id,
                    false,
                    args.reason.as_deref(),
                )),
                continuation: None,
                duration_ms: started_at.elapsed().as_millis() as u64,
                truncated: false,
            });
        }

        emit_mode_changed(
            ctx,
            "enterPlanMode",
            from_mode_id.clone(),
            to_mode_id.clone(),
        )
        .await?;

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "enterPlanMode".to_string(),
            ok: true,
            output: "entered plan mode; inspect the relevant code and tests, keep refining the \
                     session plan artifact until it is executable, then use exitPlanMode to \
                     present it for user review."
                .to_string(),
            error: None,
            metadata: Some(mode_metadata(
                &from_mode_id,
                &to_mode_id,
                true,
                args.reason.as_deref(),
            )),
            continuation: None,
            duration_ms: started_at.elapsed().as_millis() as u64,
            truncated: false,
        })
    }
}

fn mode_metadata(
    from_mode_id: &ModeId,
    to_mode_id: &ModeId,
    changed: bool,
    reason: Option<&str>,
) -> Value {
    json!({
        "schema": "modeTransition",
        "fromModeId": from_mode_id.as_str(),
        "toModeId": to_mode_id.as_str(),
        "modeChanged": changed,
        "reason": reason,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use astrcode_core::{StorageEvent, StorageEventPayload};

    use super::*;
    use crate::test_support::test_tool_context_for;

    struct RecordingSink {
        events: Arc<Mutex<Vec<StorageEvent>>>,
    }

    #[async_trait]
    impl astrcode_core::ToolEventSink for RecordingSink {
        async fn emit(&self, event: StorageEvent) -> Result<()> {
            self.events
                .lock()
                .expect("recording sink lock should work")
                .push(event);
            Ok(())
        }
    }

    #[tokio::test]
    async fn enter_plan_mode_emits_mode_change_event() {
        let tool = EnterPlanModeTool;
        let events = Arc::new(Mutex::new(Vec::new()));
        let ctx = test_tool_context_for(std::env::temp_dir())
            .with_current_mode_id(ModeId::code())
            .with_event_sink(Arc::new(RecordingSink {
                events: Arc::clone(&events),
            }));

        let result = tool
            .execute(
                "tc-enter-plan".to_string(),
                json!({ "reason": "Need a plan first" }),
                &ctx,
            )
            .await
            .expect("enterPlanMode should execute");

        assert!(result.ok);
        let events = events.lock().expect("recording sink lock should work");
        assert!(matches!(
            events.as_slice(),
            [StorageEvent {
                payload: StorageEventPayload::ModeChanged { from, to, .. },
                ..
            }] if *from == ModeId::code() && *to == ModeId::plan()
        ));
    }
}
