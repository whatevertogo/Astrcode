//! `exitPlanMode` 工具。
//!
//! 当计划已经被打磨到可执行程度时，使用该工具把 canonical plan 工件正式呈递给前端，
//! 同时把 session 切回 code mode，等待用户批准或要求修订。

use std::{fs, path::Path, time::Instant};

use astrcode_core::{AstrError, Result, SideEffect};
use astrcode_governance_contract::{
    BoundModeToolContractSnapshot, ModeArtifactDef, ModeExitGateDef, ModeId,
};
use astrcode_host_session::session_plan_content_digest;
use astrcode_tool_contract::{
    Tool, ToolCapabilityMetadata, ToolContext, ToolDefinition, ToolExecutionResult,
    ToolPromptMetadata,
};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::json;

use crate::builtin_tools::{
    mode_transition::emit_mode_changed,
    session_plan::{
        PlanArtifactContractBlockers, SessionPlanStatus, load_session_plan_state,
        persist_planning_workflow_state, persist_session_plan_state, session_plan_markdown_path,
        session_plan_paths, validate_plan_artifact_contract,
    },
};

#[derive(Default)]
pub struct ExitPlanModeTool;

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
                    "Exit plan mode after the plan is complete and executable. Requires the plan \
                     artifact to cover all required sections with concrete actionable items.",
                    "Only use `exitPlanMode` after you have inspected the code, persisted the \
                     current canonical plan artifact, and refined it until it is executable. If \
                     the plan is still vague, missing risks, or lacking verification steps, keep \
                     updating `upsertSessionPlan` instead of exiting.",
                )
                .caveat(
                    "Enforces a two-gate check: plan readiness (all sections filled, actionable \
                     items present) then a final-review checkpoint. Call again after the review \
                     to complete the exit.",
                )
                .example("{}")
                .prompt_tag("plan"),
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
        let mode_contract = require_plan_mode_contract(ctx)?;
        let artifact_contract = mode_contract.artifact.as_ref().ok_or_else(|| {
            AstrError::Validation(
                "exitPlanMode requires the current mode to declare an artifact contract"
                    .to_string(),
            )
        })?;
        let exit_gate = mode_contract.exit_gate.as_ref().ok_or_else(|| {
            AstrError::Validation(
                "exitPlanMode requires the current mode to declare an exit gate".to_string(),
            )
        })?;

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
        let blockers = validate_plan_readiness(&plan_content, artifact_contract);
        if !blockers.is_empty() {
            return Ok(review_pending_result(
                tool_call_id,
                started_at,
                &current_plan_title,
                &plan_path,
                &blockers,
                ReviewPendingKind::RevisePlan,
                exit_gate,
            ));
        }

        let plan_digest = session_plan_content_digest(plan_content.trim());
        if exit_gate.review_passes > 0
            && state.reviewed_plan_digest.as_deref() != Some(plan_digest.as_str())
        {
            // 这里故意不立刻退出 plan mode。
            // 设计目标是把“最后一次自审”保留为内部流程，而不是把 review 段落写进计划正文：
            // 当前计划版本第一次调用 exitPlanMode 只登记一个自审检查点；
            // 如果模型自审后认为计划无需再改，再次调用 exitPlanMode 才真正呈递给前端。
            // 当前 plan 专用状态只持久化“本次修订是否已经完成过 review checkpoint”，
            // 因此 builtin plan mode 的 `reviewPasses=1` 会被严格执行；更高的 review pass
            // 语义应由后续通用 mode exit 流程承载，而不是继续塞进 plan-specific 工具。
            state.reviewed_plan_digest = Some(plan_digest);
            persist_session_plan_state(&paths.state_path, &state)?;
            return Ok(review_pending_result(
                tool_call_id,
                started_at,
                &current_plan_title,
                &plan_path,
                &blockers,
                ReviewPendingKind::FinalReview,
                exit_gate,
            ));
        }

        let now = Utc::now();
        state.status = SessionPlanStatus::AwaitingApproval;
        state.updated_at = now;
        state.approved_at = None;
        persist_session_plan_state(&paths.state_path, &state)?;
        persist_planning_workflow_state(ctx, Some(&state))?;

        emit_mode_changed(ctx, "exitPlanMode", ModeId::plan(), ModeId::code()).await?;

        Ok(ToolExecutionResult {
            tool_call_id,
            tool_name: "exitPlanMode".to_string(),
            ok: true,
            output: format!(
                "Presented the canonical session plan '{}' for user review from {}.\nThe \
                 canonical plan surface already carries the full user-visible plan content.\nDo \
                 not emit any assistant summary or approval prompt after this tool result.\nStop \
                 the turn unless the canonical plan surface failed to render.\nOnly if that \
                 surface is unavailable may you send one short approval prompt.\nInternal mode \
                 transition is complete; wait for user approval or revision feedback through the \
                 canonical plan surface.",
                state.title,
                plan_path.display(),
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
                    "artifactType": artifact_contract.artifact_type,
                }
            })),
            continuation: None,
            duration_ms: started_at.elapsed().as_millis() as u64,
            truncated: false,
        })
    }
}

fn validate_plan_readiness(
    content: &str,
    artifact_contract: &ModeArtifactDef,
) -> PlanArtifactContractBlockers {
    validate_plan_artifact_contract(content, artifact_contract)
}

fn review_pending_result(
    tool_call_id: String,
    started_at: Instant,
    title: &str,
    plan_path: &Path,
    blockers: &PlanArtifactContractBlockers,
    kind: ReviewPendingKind,
    exit_gate: &ModeExitGateDef,
) -> ToolExecutionResult {
    let mut checklist = match kind {
        ReviewPendingKind::RevisePlan => vec![
            "The plan is not executable yet. Revise the canonical session plan before exiting \
             plan mode."
                .to_string(),
            "Keep this checkpoint out of user-visible assistant text; continue revising the plan \
             instead of emitting a summary paragraph."
                .to_string(),
        ],
        ReviewPendingKind::FinalReview => vec![
            "Run one internal final review before exiting plan mode. Keep that review out of the \
             plan artifact itself."
                .to_string(),
            "If the review changes the plan, persist the updated plan with upsertSessionPlan and \
             retry exitPlanMode later."
                .to_string(),
            "Do not emit the internal review as user-visible assistant text. Either revise the \
             canonical plan or call exitPlanMode again once the review is done."
                .to_string(),
        ],
    };
    checklist.push(format!(
        "Final review checklist (configured passes: {}):",
        exit_gate.review_passes
    ));
    checklist.extend(
        exit_gate
            .review_checklist
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
                "checklist": exit_gate.review_checklist,
                "reviewPasses": exit_gate.review_passes,
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

fn require_plan_mode_contract(ctx: &ToolContext) -> Result<&BoundModeToolContractSnapshot> {
    let mode_contract = ctx.bound_mode_tool_contract().ok_or_else(|| {
        AstrError::Validation(
            "exitPlanMode requires a bound mode tool contract snapshot".to_string(),
        )
    })?;
    if mode_contract.mode_id != ModeId::plan() {
        return Err(AstrError::Validation(format!(
            "exitPlanMode requires the 'plan' mode contract, got '{}'",
            mode_contract.mode_id
        )));
    }
    Ok(mode_contract)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use astrcode_core::{StorageEvent, StorageEventPayload, mode::ModeId as StoredModeId};

    use super::*;
    use crate::{
        builtin_tools::{
            session_plan::{load_workflow_state, workflow_state_path},
            upsert_session_plan::UpsertSessionPlanTool,
        },
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
                    "Scope".to_string(),
                    "Non-Goals".to_string(),
                    "Existing Code To Reuse".to_string(),
                    "Implementation Steps".to_string(),
                    "Verification".to_string(),
                    "Open Questions".to_string(),
                ],
                actionable_sections: vec![
                    "Implementation Steps".to_string(),
                    "Verification".to_string(),
                    "Open Questions".to_string(),
                ],
            }),
            exit_gate: Some(ModeExitGateDef {
                review_passes: 1,
                review_checklist: vec![
                    "Re-check assumptions against the code you already inspected.".to_string(),
                    "Look for missing edge cases, affected files, and integration boundaries."
                        .to_string(),
                    "Confirm the verification steps are specific enough to prove the change works."
                        .to_string(),
                    "If the plan changes, persist it with upsertSessionPlan before retrying \
                     exitPlanMode."
                        .to_string(),
                ],
            }),
        }
    }

    struct RecordingSink {
        events: Arc<Mutex<Vec<StorageEvent>>>,
    }

    #[async_trait]
    impl astrcode_tool_contract::ToolEventSink for RecordingSink {
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
            .with_bound_mode_tool_contract(plan_mode_contract())
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
        assert!(
            first_attempt
                .output
                .contains("Do not emit the internal review as user-visible assistant text")
        );

        let result = ExitPlanModeTool
            .execute("tc-plan-exit-2".to_string(), json!({}), &ctx)
            .await
            .expect("second exitPlanMode call should succeed");

        assert!(result.ok);
        let metadata = result.metadata.expect("metadata should exist");
        assert_eq!(metadata["schema"], json!("sessionPlanExit"));
        assert_eq!(metadata["plan"]["status"], json!("awaiting_approval"));
        assert!(result.output.contains(
            "Do not emit any assistant summary or approval prompt after this tool result"
        ));

        let state_path = session_plan_paths(&ctx)
            .expect("plan paths should resolve")
            .state_path;
        let state = load_session_plan_state(&state_path)
            .expect("state should load")
            .expect("state should exist");
        assert_eq!(state.status, SessionPlanStatus::AwaitingApproval);
        assert!(state.reviewed_plan_digest.is_some());
        let workflow =
            load_workflow_state(&workflow_state_path(&ctx).expect("workflow path should resolve"))
                .expect("workflow state should load")
                .expect("workflow state should exist");
        assert_eq!(workflow.current_phase_id, "planning");

        let events = events.lock().expect("recording sink lock should work");
        assert!(matches!(
            events.as_slice(),
            [StorageEvent {
                payload: StorageEventPayload::ModeChanged { from, to, .. },
                ..
            }] if *from == StoredModeId::plan() && *to == StoredModeId::code()
        ));
    }

    #[tokio::test]
    async fn exit_plan_mode_returns_review_pending_for_incomplete_plan() {
        let temp = tempfile::tempdir().expect("tempdir should exist");
        let upsert = UpsertSessionPlanTool;
        let ctx = test_tool_context_for(temp.path())
            .with_current_mode_id(ModeId::plan())
            .with_bound_mode_tool_contract(plan_mode_contract())
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
            json!("## Scope")
        );
        assert!(result.output.contains("not executable yet"));
        assert!(
            result
                .output
                .contains("Keep this checkpoint out of user-visible assistant text")
        );
    }

    #[test]
    fn validate_plan_readiness_accepts_plan_without_plan_review_section() {
        let content = "# Plan: Cleanup crates

## Context
- current crates are inconsistent

## Goal
- align crate boundaries

## Scope
- runtime and adapter cleanup

## Non-Goals
- change transport protocol

## Existing Code To Reuse
- reuse current capability routing

## Implementation Steps
- audit crate dependencies

## Verification
- run targeted Rust checks

## Open Questions
- none
";

        assert!(
            validate_plan_readiness(content, &plan_mode_contract().artifact.expect("artifact"))
                .is_empty()
        );
    }
}
