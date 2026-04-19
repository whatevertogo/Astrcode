//! `exitPlanMode` 工具。
//!
//! 当计划已经被打磨到可执行程度时，使用该工具把 canonical plan 工件正式呈递给前端，
//! 同时把 session 切回 code mode，等待用户批准或要求修订。

use std::{fs, path::Path, time::Instant};

use astrcode_core::{
    AstrError, ModeId, Result, SideEffect, Tool, ToolCapabilityMetadata, ToolContext,
    ToolDefinition, ToolExecutionResult, ToolPromptMetadata, session_plan_content_digest,
};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::json;

use crate::builtin_tools::{
    mode_transition::emit_mode_changed,
    session_plan::{
        SessionPlanStatus, load_session_plan_state, persist_session_plan_state,
        session_plan_markdown_path, session_plan_paths,
    },
};

#[derive(Default)]
pub struct ExitPlanModeTool;

const REQUIRED_PLAN_HEADINGS: &[&str] = &[
    "## Context",
    "## Goal",
    "## Existing Code To Reuse",
    "## Implementation Steps",
    "## Verification",
];
const FINAL_REVIEW_CHECKLIST: &[&str] = &[
    "Re-check assumptions against the code you already inspected.",
    "Look for missing edge cases, affected files, and integration boundaries.",
    "Confirm the verification steps are specific enough to prove the change works.",
    "If the plan changes, persist it with upsertSessionPlan before retrying exitPlanMode.",
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlanExitBlockers {
    missing_headings: Vec<String>,
    invalid_sections: Vec<String>,
}

impl PlanExitBlockers {
    fn is_empty(&self) -> bool {
        self.missing_headings.is_empty() && self.invalid_sections.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReviewPendingKind {
    RevisePlan,
    FinalReview,
}

#[async_trait]
impl Tool for ExitPlanModeTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "exitPlanMode".to_string(),
            description: "Present the current session plan to the user and switch back to code \
                          mode."
                .to_string(),
            parameters: json!({
                "type": "object",
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
                    "Present the current session plan to the user and leave plan mode.",
                    "Only use `exitPlanMode` after you have inspected the code, persisted the \
                     current canonical plan artifact, and refined it until it is executable. If \
                     the plan is still vague, missing risks, or lacking verification steps, keep \
                     updating `upsertSessionPlan` instead of exiting.",
                )
                .caveat(
                    "`exitPlanMode` first checks whether the current plan is executable, then \
                     enforces one internal final-review checkpoint before it actually exits. Keep \
                     that review out of the plan artifact itself unless the user explicitly asks \
                     for it.",
                )
                .example("{}")
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
        let started_at = Instant::now();
        if !args.is_null() && args != json!({}) {
            return Err(AstrError::Validation(
                "exitPlanMode does not accept arguments".to_string(),
            ));
        }

        if ctx.current_mode_id() != &ModeId::plan() {
            return Err(AstrError::Validation(format!(
                "exitPlanMode can only be called from plan mode, current mode is '{}'",
                ctx.current_mode_id()
            )));
        }

        let paths = session_plan_paths(ctx)?;
        let Some(mut state) = load_session_plan_state(&paths.state_path)? else {
            return Err(AstrError::Validation(
                "cannot exit plan mode because no session plan artifact exists yet".to_string(),
            ));
        };

        let current_plan_slug = state.active_plan_slug.clone();
        let current_plan_title = state.title.clone();
        let plan_path = session_plan_markdown_path(&paths.plan_dir, &current_plan_slug);
        let plan_content = fs::read_to_string(&plan_path).map_err(|error| {
            AstrError::io(
                format!("failed reading session plan file '{}'", plan_path.display()),
                error,
            )
        })?;
        let blockers = validate_plan_readiness(&plan_content);
        if !blockers.is_empty() {
            return Ok(review_pending_result(
                tool_call_id,
                started_at,
                &current_plan_title,
                &plan_path,
                &blockers,
                ReviewPendingKind::RevisePlan,
            ));
        }

        let plan_digest = session_plan_content_digest(plan_content.trim());
        if state.reviewed_plan_digest.as_deref() != Some(plan_digest.as_str()) {
            // 这里故意不立刻退出 plan mode。
            // 设计目标是把“最后一次自审”保留为内部流程，而不是把 review 段落写进计划正文：
            // 当前计划版本第一次调用 exitPlanMode 只登记一个自审检查点；
            // 如果模型自审后认为计划无需再改，再次调用 exitPlanMode 才真正呈递给前端。
            state.reviewed_plan_digest = Some(plan_digest);
            persist_session_plan_state(&paths.state_path, &state)?;
            return Ok(review_pending_result(
                tool_call_id,
                started_at,
                &current_plan_title,
                &plan_path,
                &blockers,
                ReviewPendingKind::FinalReview,
            ));
        }

        let now = Utc::now();
        state.status = SessionPlanStatus::AwaitingApproval;
        state.updated_at = now;
        state.approved_at = None;
        persist_session_plan_state(&paths.state_path, &state)?;

        emit_mode_changed(ctx, "exitPlanMode", ModeId::plan(), ModeId::code()).await?;

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "exitPlanMode".to_string(),
            ok: true,
            output: format!(
                "presented the session plan '{}' for user review from {}.\n\n{}",
                state.title,
                plan_path.display(),
                plan_content.trim()
            ),
            error: None,
            metadata: Some(json!({
                "schema": "sessionPlanExit",
                "mode": {
                    "fromModeId": "plan",
                    "toModeId": "code",
                    "modeChanged": true,
                },
                "plan": {
                    "title": state.title,
                    "status": state.status.as_str(),
                    "slug": current_plan_slug,
                    "planPath": plan_path.to_string_lossy(),
                    "content": plan_content.trim(),
                    "updatedAt": state.updated_at.to_rfc3339(),
                }
            })),
            continuation: None,
            duration_ms: started_at.elapsed().as_millis() as u64,
            truncated: false,
        })
    }
}

fn validate_plan_readiness(content: &str) -> PlanExitBlockers {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return PlanExitBlockers {
            missing_headings: REQUIRED_PLAN_HEADINGS
                .iter()
                .map(|heading| (*heading).to_string())
                .collect(),
            invalid_sections: Vec::new(),
        };
    }

    let missing_headings = REQUIRED_PLAN_HEADINGS
        .iter()
        .copied()
        .filter(|heading| !trimmed.contains(heading))
        .map(str::to_string)
        .collect::<Vec<_>>();

    let mut invalid_sections = Vec::new();
    if let Err(error) = ensure_actionable_section(trimmed, "## Implementation Steps") {
        invalid_sections.push(error);
    }
    if let Err(error) = ensure_actionable_section(trimmed, "## Verification") {
        invalid_sections.push(error);
    }

    PlanExitBlockers {
        missing_headings,
        invalid_sections,
    }
}

fn ensure_actionable_section(content: &str, heading: &str) -> std::result::Result<(), String> {
    let section = section_body(content, heading)
        .ok_or_else(|| format!("session plan is missing required section '{}'", heading))?;
    let has_actionable_line = section.lines().map(str::trim).any(|line| {
        !line.is_empty()
            && (line.starts_with("- ")
                || line.starts_with("* ")
                || line.chars().next().is_some_and(|ch| ch.is_ascii_digit()))
    });
    if has_actionable_line {
        return Ok(());
    }
    Err(format!(
        "session plan section '{}' must contain concrete actionable items before exiting plan mode",
        heading
    ))
}

fn section_body<'a>(content: &'a str, heading: &str) -> Option<&'a str> {
    let start = content.find(heading)?;
    let after_heading = &content[start + heading.len()..];
    let next_heading_offset = after_heading.find("\n## ");
    Some(match next_heading_offset {
        Some(offset) => &after_heading[..offset],
        None => after_heading,
    })
}

fn review_pending_result(
    tool_call_id: String,
    started_at: Instant,
    title: &str,
    plan_path: &Path,
    blockers: &PlanExitBlockers,
    kind: ReviewPendingKind,
) -> ToolExecutionResult {
    let mut checklist = match kind {
        ReviewPendingKind::RevisePlan => vec![
            "The plan is not executable yet. Revise the canonical session plan before exiting \
             plan mode."
                .to_string(),
        ],
        ReviewPendingKind::FinalReview => vec![
            "Run one internal final review before exiting plan mode. Keep that review out of the \
             plan artifact itself."
                .to_string(),
            "If the review changes the plan, persist the updated plan with upsertSessionPlan and \
             retry exitPlanMode later."
                .to_string(),
        ],
    };
    checklist.push("Final review checklist:".to_string());
    checklist.extend(
        FINAL_REVIEW_CHECKLIST
            .iter()
            .enumerate()
            .map(|(index, item)| format!("{}. {}", index + 1, item)),
    );

    if !blockers.missing_headings.is_empty() {
        checklist.push(format!(
            "Missing sections: {}",
            blockers.missing_headings.join(", ")
        ));
    }
    if !blockers.invalid_sections.is_empty() {
        checklist.push(format!(
            "Sections to strengthen: {}",
            blockers.invalid_sections.join("; ")
        ));
    }

    ToolExecutionResult {
        tool_call_id,
        tool_name: "exitPlanMode".to_string(),
        ok: true,
        output: checklist.join("\n"),
        error: None,
        metadata: Some(json!({
            "schema": "sessionPlanExitReviewPending",
            "plan": {
                "title": title,
                "planPath": plan_path.to_string_lossy(),
            },
            "review": {
                "kind": match kind {
                    ReviewPendingKind::RevisePlan => "revise_plan",
                    ReviewPendingKind::FinalReview => "final_review",
                },
                "checklist": FINAL_REVIEW_CHECKLIST,
            },
            "blockers": {
                "missingHeadings": blockers.missing_headings,
                "invalidSections": blockers.invalid_sections,
            }
        })),
        continuation: None,
        duration_ms: started_at.elapsed().as_millis() as u64,
        truncated: false,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use astrcode_core::{StorageEvent, StorageEventPayload};

    use super::*;
    use crate::{
        builtin_tools::upsert_session_plan::UpsertSessionPlanTool,
        test_support::test_tool_context_for,
    };

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
    async fn exit_plan_mode_requires_internal_review_before_presenting_plan() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let upsert = UpsertSessionPlanTool;
        let events = Arc::new(Mutex::new(Vec::new()));
        let ctx = test_tool_context_for(temp.path())
            .with_current_mode_id(ModeId::plan())
            .with_event_sink(Arc::new(RecordingSink {
                events: Arc::clone(&events),
            }));

        upsert
            .execute(
                "tc-plan-seed".to_string(),
                json!({
                    "title": "Cleanup crates",
                    "status": "draft",
                    "content": "# Plan: Cleanup crates\n\n## Context\n- current crates are inconsistent\n\n## Goal\n- align crate boundaries\n\n## Scope\n- runtime and adapter cleanup\n\n## Non-Goals\n- change transport protocol\n\n## Existing Code To Reuse\n- reuse current capability routing\n\n## Implementation Steps\n- audit crate dependencies\n- introduce shared plan tools\n\n## Verification\n- run targeted Rust and frontend checks\n\n## Open Questions\n- none"
                }),
                &ctx,
            )
            .await
            .expect("seed plan should succeed");

        let first_attempt = ExitPlanModeTool
            .execute("tc-plan-exit-1".to_string(), json!({}), &ctx)
            .await
            .expect("first exitPlanMode call should return review pending");

        assert!(first_attempt.ok);
        let first_metadata = first_attempt.metadata.expect("metadata should exist");
        assert_eq!(
            first_metadata["schema"],
            json!("sessionPlanExitReviewPending")
        );
        assert_eq!(first_metadata["review"]["kind"], json!("final_review"));

        let result = ExitPlanModeTool
            .execute("tc-plan-exit-2".to_string(), json!({}), &ctx)
            .await
            .expect("second exitPlanMode call should succeed");

        assert!(result.ok);
        let metadata = result.metadata.expect("metadata should exist");
        assert_eq!(metadata["schema"], json!("sessionPlanExit"));
        assert_eq!(metadata["plan"]["status"], json!("awaiting_approval"));

        let state_path = session_plan_paths(&ctx)
            .expect("plan paths should resolve")
            .state_path;
        let state = load_session_plan_state(&state_path)
            .expect("state should load")
            .expect("state should exist");
        assert_eq!(state.status, SessionPlanStatus::AwaitingApproval);
        assert!(state.reviewed_plan_digest.is_some());

        let events = events.lock().expect("recording sink lock should work");
        assert!(matches!(
            events.as_slice(),
            [StorageEvent {
                payload: StorageEventPayload::ModeChanged { from, to, .. },
                ..
            }] if *from == ModeId::plan() && *to == ModeId::code()
        ));
    }

    #[tokio::test]
    async fn exit_plan_mode_returns_review_pending_for_incomplete_plan() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let upsert = UpsertSessionPlanTool;
        let ctx = test_tool_context_for(temp.path())
            .with_current_mode_id(ModeId::plan())
            .with_event_sink(Arc::new(RecordingSink {
                events: Arc::new(Mutex::new(Vec::new())),
            }));

        upsert
            .execute(
                "tc-plan-seed".to_string(),
                json!({
                    "title": "Cleanup crates",
                    "status": "draft",
                    "content": "# Plan: Cleanup crates\n\n## Context\n- current crates are inconsistent\n\n## Goal\n- align crate boundaries\n\n## Implementation Steps\n- audit crate dependencies\n\n## Verification\nrun targeted Rust checks"
                }),
                &ctx,
            )
            .await
            .expect("seed plan should succeed");

        let result = ExitPlanModeTool
            .execute("tc-plan-exit".to_string(), json!({}), &ctx)
            .await
            .expect("tool should return a review-pending result");

        assert!(result.ok);
        let metadata = result.metadata.expect("metadata should exist");
        assert_eq!(metadata["schema"], json!("sessionPlanExitReviewPending"));
        assert_eq!(metadata["review"]["kind"], json!("revise_plan"));
        assert_eq!(
            metadata["blockers"]["missingHeadings"][0],
            json!("## Existing Code To Reuse")
        );
        assert!(result.output.contains("not executable yet"));
    }

    #[test]
    fn validate_plan_readiness_accepts_plan_without_plan_review_section() {
        let content = "# Plan: Cleanup crates

## Context
- current crates are inconsistent

## Goal
- align crate boundaries

## Existing Code To Reuse
- reuse current capability routing

## Implementation Steps
- audit crate dependencies

## Verification
- run targeted Rust checks
";

        assert!(validate_plan_readiness(content).is_empty());
    }
}
