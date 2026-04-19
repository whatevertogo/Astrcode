//! `upsertSessionPlan` 工具。
//!
//! 该工具只允许写当前 session 下的 `plan/` 目录和 `state.json`，
//! 作为 plan mode 唯一的受限写入口。

use std::{fs, path::PathBuf, time::Instant};

use astrcode_core::{
    AstrError, Result, SideEffect, Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition,
    ToolExecutionResult, ToolPromptMetadata,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::builtin_tools::fs_common::{check_cancel, session_dir_for_tool_results};

const PLAN_DIR_NAME: &str = "plan";
const PLAN_STATE_FILE_NAME: &str = "state.json";
const PLAN_PATH_TIMESTAMP_FORMAT: &str = "%Y%m%dT%H%M%SZ";

#[derive(Default)]
pub struct UpsertSessionPlanTool;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SessionPlanStatus {
    Draft,
    AwaitingApproval,
    Approved,
    Superseded,
}

impl SessionPlanStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Approved => "approved",
            Self::Superseded => "superseded",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionPlanState {
    active_plan_slug: String,
    title: String,
    status: SessionPlanStatus,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    approved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpsertSessionPlanArgs {
    title: String,
    content: String,
    #[serde(default)]
    topic: Option<String>,
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    status: Option<SessionPlanStatus>,
}

#[async_trait]
impl Tool for UpsertSessionPlanTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "upsertSessionPlan".to_string(),
            description: "Create or overwrite the current session's plan artifact and state file."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Human-readable plan title."
                    },
                    "content": {
                        "type": "string",
                        "description": "Full markdown body to persist into the session plan file."
                    },
                    "topic": {
                        "type": "string",
                        "description": "Optional task/topic text used to derive a slug when no active plan exists yet."
                    },
                    "slug": {
                        "type": "string",
                        "description": "Optional explicit kebab-case slug. When omitted, the tool reuses the active session slug or derives one from topic/title."
                    },
                    "status": {
                        "type": "string",
                        "enum": ["draft", "awaiting_approval", "approved", "superseded"],
                        "description": "Plan state to persist alongside the markdown artifact."
                    }
                },
                "required": ["title", "content"],
                "additionalProperties": false
            }),
        }
    }

    fn capability_metadata(&self) -> ToolCapabilityMetadata {
        ToolCapabilityMetadata::builtin()
            .tags(["filesystem", "write", "plan"])
            .permission("filesystem.write")
            .side_effect(SideEffect::Local)
            .prompt(
                ToolPromptMetadata::new(
                    "Create or update the current session's plan artifact.",
                    "Use `upsertSessionPlan` when plan mode needs to persist the canonical \
                     session plan markdown and its `state.json` metadata. This tool can only \
                     write inside the current session's `plan/` directory.",
                )
                .caveat(
                    "This is the only write tool available in plan mode. It overwrites the whole \
                     plan file content each time.",
                )
                .example(
                    "{ title: \"Cleanup crates\", slug: \"cleanup-crates\", content: \"# Plan: \
                     Cleanup crates\\n...\", status: \"draft\" }",
                )
                .prompt_tag("plan")
                .always_include(true),
            )
    }

    async fn execute(
        &self,
        tool_call_id: String,
        args: serde_json::Value,
        ctx: &ToolContext,
    ) -> Result<ToolExecutionResult> {
        check_cancel(ctx.cancel())?;

        let args: UpsertSessionPlanArgs = serde_json::from_value(args)
            .map_err(|error| AstrError::parse("invalid args for upsertSessionPlan", error))?;
        let title = args.title.trim();
        if title.is_empty() {
            return Err(AstrError::Validation(
                "plan title must not be empty".to_string(),
            ));
        }
        let content = args.content.trim();
        if content.is_empty() {
            return Err(AstrError::Validation(
                "plan markdown content must not be empty".to_string(),
            ));
        }

        let started_at = Instant::now();
        let plan_dir = session_dir_for_tool_results(ctx)?.join(PLAN_DIR_NAME);
        let state_path = plan_dir.join(PLAN_STATE_FILE_NAME);
        let previous_state = load_state(&state_path)?;
        let slug = resolve_slug(&args, previous_state.as_ref());
        let plan_path = plan_dir.join(format!("{slug}.md"));
        let now = Utc::now();
        let status = args.status.unwrap_or(SessionPlanStatus::Draft);
        let created_at = previous_state
            .as_ref()
            .filter(|state| state.active_plan_slug == slug)
            .map(|state| state.created_at)
            .unwrap_or(now);
        let approved_at = if matches!(status, SessionPlanStatus::Approved) {
            previous_state
                .as_ref()
                .and_then(|state| state.approved_at)
                .or(Some(now))
        } else {
            None
        };
        let state = SessionPlanState {
            active_plan_slug: slug.clone(),
            title: title.to_string(),
            status,
            created_at,
            updated_at: now,
            approved_at,
        };

        fs::create_dir_all(&plan_dir).map_err(|error| {
            AstrError::io(
                format!(
                    "failed creating session plan directory '{}'",
                    plan_dir.display()
                ),
                error,
            )
        })?;
        fs::write(&plan_path, format!("{content}\n")).map_err(|error| {
            AstrError::io(
                format!("failed writing session plan file '{}'", plan_path.display()),
                error,
            )
        })?;
        let state_content = serde_json::to_string_pretty(&state)
            .map_err(|error| AstrError::parse("failed to serialize session plan state", error))?;
        fs::write(&state_path, state_content).map_err(|error| {
            AstrError::io(
                format!(
                    "failed writing session plan state '{}'",
                    state_path.display()
                ),
                error,
            )
        })?;

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "upsertSessionPlan".to_string(),
            ok: true,
            output: format!(
                "updated session plan '{}' at {}",
                title,
                plan_path.display()
            ),
            error: None,
            metadata: Some(json!({
                "planPath": plan_path.to_string_lossy(),
                "slug": slug,
                "status": state.status.as_str(),
                "title": state.title,
                "updatedAt": state.updated_at.to_rfc3339(),
            })),
            child_ref: None,
            duration_ms: started_at.elapsed().as_millis() as u64,
            truncated: false,
        })
    }
}

fn load_state(path: &PathBuf) -> Result<Option<SessionPlanState>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path)
        .map_err(|error| AstrError::io(format!("failed reading '{}'", path.display()), error))?;
    let state = serde_json::from_str::<SessionPlanState>(&content)
        .map_err(|error| AstrError::parse("failed to parse session plan state", error))?;
    Ok(Some(state))
}

fn resolve_slug(args: &UpsertSessionPlanArgs, previous_state: Option<&SessionPlanState>) -> String {
    if let Some(slug) = args.slug.as_deref().and_then(normalize_slug) {
        return slug;
    }
    if let Some(previous_state) = previous_state {
        return previous_state.active_plan_slug.clone();
    }
    args.topic
        .as_deref()
        .and_then(slugify)
        .or_else(|| slugify(&args.title))
        .unwrap_or_else(|| format!("plan-{}", Utc::now().format(PLAN_PATH_TIMESTAMP_FORMAT)))
}

fn normalize_slug(input: &str) -> Option<String> {
    let mut normalized = String::new();
    let mut last_dash = false;
    for ch in input.chars().map(|ch| ch.to_ascii_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch);
            last_dash = false;
            continue;
        }
        if (ch == '-' || ch == '_' || ch.is_whitespace()) && !last_dash && !normalized.is_empty() {
            normalized.push('-');
            last_dash = true;
        }
    }
    let normalized = normalized.trim_matches('-').to_string();
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn slugify(input: &str) -> Option<String> {
    normalize_slug(input)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::test_support::test_tool_context_for;

    #[tokio::test]
    async fn upsert_session_plan_creates_markdown_and_state() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let tool = UpsertSessionPlanTool;
        let result = tool
            .execute(
                "tc-plan-create".to_string(),
                json!({
                    "title": "Cleanup crates",
                    "content": "# Plan: Cleanup crates\n\n## Context",
                    "slug": "cleanup-crates",
                    "status": "draft"
                }),
                &test_tool_context_for(temp.path()),
            )
            .await
            .expect("tool should execute");

        assert!(result.ok);
        let plan_dir = temp
            .path()
            .join(".astrcode-test-state")
            .join("sessions")
            .join("session-test")
            .join("plan");
        assert!(plan_dir.join("cleanup-crates.md").exists());
        assert!(plan_dir.join("state.json").exists());
        assert_eq!(
            result.metadata.expect("metadata should exist")["slug"],
            json!("cleanup-crates")
        );
    }

    #[tokio::test]
    async fn upsert_session_plan_reuses_existing_slug_when_omitted() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let tool = UpsertSessionPlanTool;
        let ctx = test_tool_context_for(temp.path());

        tool.execute(
            "tc-plan-initial".to_string(),
            json!({
                "title": "Cleanup crates",
                "content": "# Plan: Cleanup crates",
                "slug": "cleanup-crates",
                "status": "draft"
            }),
            &ctx,
        )
        .await
        .expect("initial write should work");

        let result = tool
            .execute(
                "tc-plan-update".to_string(),
                json!({
                    "title": "Cleanup crates revised",
                    "content": "# Plan: Cleanup crates revised",
                    "status": "awaiting_approval"
                }),
                &ctx,
            )
            .await
            .expect("update should execute");

        assert!(result.ok);
        assert_eq!(
            result.metadata.expect("metadata should exist")["slug"],
            json!("cleanup-crates")
        );
    }
}
