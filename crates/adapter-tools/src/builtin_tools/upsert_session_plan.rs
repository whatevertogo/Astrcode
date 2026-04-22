//! `upsertSessionPlan` 工具。
//!
//! 该工具只允许写当前 session 下的 `plan/` 目录和 `state.json`，
//! 作为 canonical session plan 的唯一受限写入口。

use std::{fs, time::Instant};

use astrcode_core::{
    AstrError, Result, SessionPlanState, SessionPlanStatus, SideEffect, Tool,
    ToolCapabilityMetadata, ToolContext, ToolDefinition, ToolExecutionResult, ToolPromptMetadata,
};
use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;
use serde_json::json;

use crate::builtin_tools::{
    fs_common::check_cancel,
    session_plan::{
        PLAN_PATH_TIMESTAMP_FORMAT, load_session_plan_state, persist_planning_workflow_state,
        persist_session_plan_state, session_plan_markdown_path, session_plan_paths,
        validate_plan_artifact_contract,
    },
};

#[derive(Default)]
pub struct UpsertSessionPlanTool;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpsertSessionPlanArgs {
    title: String,
    content: String,
    #[serde(default)]
    status: Option<SessionPlanStatus>,
}

#[async_trait]
impl Tool for UpsertSessionPlanTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "upsertSessionPlan".to_string(),
            description: "Create or overwrite the canonical session plan artifact and its state."
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
                        "description": "Full markdown body to persist into the canonical session plan file."
                    },
                    "status": {
                        "type": "string",
                        "enum": ["draft", "awaiting_approval"],
                        "description": "Plan state to persist alongside the markdown artifact. \
                                        Terminal status transitions are not written through this tool."
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
                    "Create or update the canonical session plan artifact.",
                    "Use `upsertSessionPlan` when plan mode needs to persist the canonical \
                     session plan markdown and its `state.json`. This tool is the only supported \
                     writer for `sessions/<id>/plan/**`.",
                )
                .caveat(
                    "A session has exactly one canonical plan. Revise that plan for the same \
                     task; if the task changes, overwrite the current canonical plan instead of \
                     creating another one.",
                )
                .example(
                    "{ title: \"Cleanup crates\", content: \"# Plan: Cleanup crates\\n...\", \
                     status: \"draft\" }",
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
        let mode_contract = ctx.bound_mode_tool_contract().ok_or_else(|| {
            AstrError::Validation(
                "upsertSessionPlan requires a bound mode tool contract snapshot".to_string(),
            )
        })?;
        let artifact_contract = mode_contract.artifact.as_ref().ok_or_else(|| {
            AstrError::Validation(
                "upsertSessionPlan requires the current mode to declare an artifact contract"
                    .to_string(),
            )
        })?;

        let started_at = Instant::now();
        let paths = session_plan_paths(ctx)?;
        let now = Utc::now();
        let existing = load_session_plan_state(&paths.state_path)?;
        let slug = existing
            .as_ref()
            .map(|state| state.active_plan_slug.clone())
            .or_else(|| slugify(&args.title))
            .unwrap_or_else(|| format!("plan-{}", Utc::now().format(PLAN_PATH_TIMESTAMP_FORMAT)));
        let plan_path = session_plan_markdown_path(&paths.plan_dir, &slug);
        let status = args.status.unwrap_or(SessionPlanStatus::Draft);
        if !matches!(
            status,
            SessionPlanStatus::Draft | SessionPlanStatus::AwaitingApproval
        ) {
            // 用户批准/完成/替换都属于受控状态迁移，不能让模型通过写 plan 伪造。
            return Err(AstrError::Validation(format!(
                "upsertSessionPlan must not persist terminal status '{}'; only 'draft' or \
                 'awaiting_approval' are allowed",
                status.as_str()
            )));
        }
        if matches!(
            status,
            SessionPlanStatus::AwaitingApproval | SessionPlanStatus::Completed
        ) {
            let blockers = validate_plan_artifact_contract(content, artifact_contract);
            if !blockers.is_empty() {
                return Err(AstrError::Validation(format!(
                    "session plan does not satisfy artifact contract '{}': missing headings [{}], \
                     invalid sections [{}]",
                    artifact_contract.artifact_type,
                    blockers.missing_headings.join(", "),
                    blockers.invalid_sections.join("; "),
                )));
            }
        }

        fs::create_dir_all(&paths.plan_dir).map_err(|error| {
            AstrError::io(
                format!(
                    "failed creating session plan directory '{}'",
                    paths.plan_dir.display()
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

        let state = SessionPlanState {
            active_plan_slug: slug.clone(),
            title: title.to_string(),
            status: status.clone(),
            created_at: existing
                .as_ref()
                .map(|state| state.created_at)
                .unwrap_or(now),
            updated_at: now,
            reviewed_plan_digest: None,
            approved_at: None,
            archived_plan_digest: existing
                .as_ref()
                .and_then(|state| state.archived_plan_digest.clone()),
            archived_at: existing.as_ref().and_then(|state| state.archived_at),
        };
        persist_session_plan_state(&paths.state_path, &state)?;
        if ctx.current_mode_id() == &astrcode_core::ModeId::plan() {
            persist_planning_workflow_state(ctx, Some(&state))?;
        }

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
                "artifactType": artifact_contract.artifact_type,
                "requiredHeadings": artifact_contract.required_headings,
            })),
            continuation: None,
            duration_ms: started_at.elapsed().as_millis() as u64,
            truncated: false,
        })
    }
}

fn slugify(input: &str) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use astrcode_core::{BoundModeToolContractSnapshot, ModeArtifactDef, ModeExitGateDef, ModeId};
    use serde_json::json;

    use super::*;
    use crate::{
        builtin_tools::session_plan::{load_workflow_state, workflow_state_path},
        test_support::test_tool_context_for,
    };

    fn plan_mode_contract() -> BoundModeToolContractSnapshot {
        BoundModeToolContractSnapshot {
            mode_id: ModeId::plan(),
            artifact: Some(ModeArtifactDef {
                artifact_type: "canonical-plan".to_string(),
                file_template: None,
                schema_template: None,
                required_headings: vec![
                    "Context".to_string(),
                    "Goal".to_string(),
                    "Implementation Steps".to_string(),
                    "Verification".to_string(),
                ],
                actionable_sections: vec![
                    "Implementation Steps".to_string(),
                    "Verification".to_string(),
                ],
            }),
            exit_gate: Some(ModeExitGateDef {
                review_passes: 1,
                review_checklist: vec!["Check the plan".to_string()],
            }),
        }
    }

    #[tokio::test]
    async fn upsert_session_plan_creates_canonical_plan_state() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let tool = UpsertSessionPlanTool;
        let ctx = test_tool_context_for(temp.path())
            .with_current_mode_id(ModeId::plan())
            .with_bound_mode_tool_contract(plan_mode_contract());
        let result = tool
            .execute(
                "tc-plan-create".to_string(),
                json!({
                    "title": "Cleanup crates",
                    "content": "# Plan: Cleanup crates\n\n## Context\n- current crates are inconsistent\n\n## Goal\n- align crate boundaries\n\n## Implementation Steps\n- audit crate boundaries\n\n## Verification\n- run targeted tests",
                    "status": "draft"
                }),
                &ctx,
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
        let metadata = result.metadata.expect("metadata should exist");
        let slug = metadata["slug"].as_str().expect("slug should exist");
        assert!(plan_dir.join(format!("{slug}.md")).exists());
        assert!(plan_dir.join("state.json").exists());
        let workflow =
            load_workflow_state(&workflow_state_path(&ctx).expect("workflow path should resolve"))
                .expect("workflow state should load")
                .expect("workflow state should exist");
        assert_eq!(workflow.current_phase_id, "planning");
        assert_eq!(
            workflow
                .artifact_refs
                .get("canonical-plan")
                .expect("canonical plan artifact should exist")
                .path,
            plan_dir.join(format!("{slug}.md")).display().to_string()
        );
    }

    #[tokio::test]
    async fn upsert_session_plan_reuses_existing_slug() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let tool = UpsertSessionPlanTool;
        let ctx =
            test_tool_context_for(temp.path()).with_bound_mode_tool_contract(plan_mode_contract());

        let first = tool
            .execute(
                "tc-plan-initial".to_string(),
                json!({
                    "title": "Cleanup crates",
                    "content": "# Plan: Cleanup crates\n\n## Context\n- current crates are inconsistent\n\n## Goal\n- align crate boundaries\n\n## Implementation Steps\n- audit crate boundaries\n\n## Verification\n- run targeted tests",
                    "status": "draft"
                }),
                &ctx,
            )
            .await
            .expect("initial write should work");
        let first_slug = first
            .metadata
            .as_ref()
            .and_then(|metadata| metadata["slug"].as_str())
            .expect("slug should exist")
            .to_string();

        let result = tool
            .execute(
                "tc-plan-update".to_string(),
                json!({
                    "title": "Cleanup crates revised",
                    "content": "# Plan: Cleanup crates revised\n\n## Context\n- current crates are inconsistent\n\n## Goal\n- align crate boundaries\n\n## Implementation Steps\n- audit crate boundaries\n\n## Verification\n- run targeted tests",
                    "status": "awaiting_approval"
                }),
                &ctx,
            )
            .await
            .expect("update should execute");

        assert!(result.ok);
        assert_eq!(
            result.metadata.expect("metadata should exist")["slug"],
            json!(first_slug)
        );
    }

    #[tokio::test]
    async fn upsert_session_plan_preserves_archive_markers() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let tool = UpsertSessionPlanTool;
        let ctx =
            test_tool_context_for(temp.path()).with_bound_mode_tool_contract(plan_mode_contract());

        tool.execute(
            "tc-plan-first".to_string(),
            json!({
                "title": "Cleanup crates",
                "content": "# Plan: Cleanup crates\n\n## Context\n- current crates are inconsistent\n\n## Goal\n- align crate boundaries\n\n## Implementation Steps\n- audit crate boundaries\n\n## Verification\n- run targeted tests",
                "status": "awaiting_approval"
            }),
            &ctx,
        )
        .await
        .expect("first plan should work");

        let state_path = session_plan_paths(&ctx)
            .expect("plan paths should resolve")
            .state_path;
        let mut state = load_session_plan_state(&state_path)
            .expect("state should load")
            .expect("state should exist");
        state.status = SessionPlanStatus::Approved;
        state.approved_at = Some(Utc::now());
        state.archived_plan_digest = Some("digest-a".to_string());
        state.archived_at = Some(Utc::now());
        persist_session_plan_state(&state_path, &state).expect("state should persist");

        tool.execute(
            "tc-plan-second".to_string(),
            json!({
                "title": "Cleanup crates revised",
                "content": "# Plan: Cleanup crates revised\n\n## Context\n- current crates are inconsistent\n\n## Goal\n- align crate boundaries\n\n## Implementation Steps\n- audit crate boundaries\n\n## Verification\n- run targeted tests",
                "status": "draft"
            }),
            &ctx,
        )
        .await
        .expect("second write should work");

        let state = load_session_plan_state(&state_path)
            .expect("state should load")
            .expect("state should exist");
        assert_eq!(state.archived_plan_digest.as_deref(), Some("digest-a"));
        assert!(state.reviewed_plan_digest.is_none());
        assert!(state.approved_at.is_none());
    }

    #[tokio::test]
    async fn upsert_session_plan_rejects_terminal_statuses_before_user_approval() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let tool = UpsertSessionPlanTool;
        let ctx =
            test_tool_context_for(temp.path()).with_bound_mode_tool_contract(plan_mode_contract());

        for status in ["approved", "completed", "superseded"] {
            let error = tool
                .execute(
                    format!("tc-plan-{status}"),
                    json!({
                        "title": "Cleanup crates",
                        "content": "# Plan: Cleanup crates\n\n## Context\n- current crates are inconsistent\n\n## Goal\n- align crate boundaries\n\n## Implementation Steps\n- audit crate boundaries\n\n## Verification\n- run targeted tests",
                        "status": status
                    }),
                    &ctx,
                )
                .await
                .unwrap_err();

            assert!(
                error
                    .to_string()
                    .contains("must not persist terminal status")
            );
        }
    }

    #[tokio::test]
    async fn upsert_session_plan_preserves_existing_custom_slug_from_state() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let tool = UpsertSessionPlanTool;
        let ctx =
            test_tool_context_for(temp.path()).with_bound_mode_tool_contract(plan_mode_contract());
        let paths = session_plan_paths(&ctx).expect("plan paths should resolve");
        let now = Utc::now();
        let existing_slug = "my-custom-slug".to_string();

        persist_session_plan_state(
            &paths.state_path,
            &SessionPlanState {
                active_plan_slug: existing_slug.clone(),
                title: "Existing title".to_string(),
                status: SessionPlanStatus::Draft,
                created_at: now,
                updated_at: now,
                reviewed_plan_digest: None,
                approved_at: None,
                archived_plan_digest: None,
                archived_at: None,
            },
        )
        .expect("existing state should persist");

        let result = tool
            .execute(
                "tc-plan-custom-slug".to_string(),
                json!({
                    "title": "Completely different title",
                    "content": "# Plan: revised\n\n## Context\n- current crates are inconsistent\n\n## Goal\n- align crate boundaries\n\n## Implementation Steps\n- audit crate boundaries\n\n## Verification\n- run targeted tests",
                    "status": "draft"
                }),
                &ctx,
            )
            .await
            .expect("update should execute");

        assert!(result.ok);
        assert_eq!(
            result.metadata.expect("metadata should exist")["slug"],
            json!(existing_slug)
        );
        assert!(paths.plan_dir.join("my-custom-slug.md").exists());
    }

    #[tokio::test]
    async fn upsert_session_plan_rejects_reviewable_status_when_contract_is_unmet() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let tool = UpsertSessionPlanTool;
        let ctx =
            test_tool_context_for(temp.path()).with_bound_mode_tool_contract(plan_mode_contract());

        let error = tool
            .execute(
                "tc-plan-invalid".to_string(),
                json!({
                    "title": "Cleanup crates",
                    "content": "# Plan: Cleanup crates\n\n## Context\n- grounded enough",
                    "status": "awaiting_approval"
                }),
                &ctx,
            )
            .await
            .expect_err("reviewable status should enforce artifact contract");

        assert!(error.to_string().contains("artifact contract"));
    }
}
