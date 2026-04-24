//! session 计划工件的共享读写辅助。
//!
//! `upsertSessionPlan`、`exitPlanMode` 等工具都需要读写同一份单 plan 状态；
//! 这里集中维护状态结构和路径规则，避免多处各自漂移。

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use astrcode_core::{AstrError, Result};
use astrcode_governance_contract::ModeArtifactDef;
pub use astrcode_host_session::{SessionPlanState, SessionPlanStatus};
use astrcode_host_session::{
    WorkflowArtifactRef, WorkflowInstanceState, session_plan_content_digest,
};
use astrcode_tool_contract::ToolContext;
use chrono::Utc;

use crate::builtin_tools::fs_common::session_dir_for_tool_results;

pub const PLAN_DIR_NAME: &str = "plan";
pub const PLAN_STATE_FILE_NAME: &str = "state.json";
pub const PLAN_PATH_TIMESTAMP_FORMAT: &str = "%Y%m%dT%H%M%SZ";
pub const WORKFLOW_DIR_NAME: &str = "workflow";
pub const WORKFLOW_STATE_FILE_NAME: &str = "state.json";
pub const PLAN_EXECUTE_WORKFLOW_ID: &str = "plan_execute";
pub const PLANNING_PHASE_ID: &str = "planning";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlanArtifactContractBlockers {
    pub missing_headings: Vec<String>,
    pub invalid_sections: Vec<String>,
}

impl PlanArtifactContractBlockers {
    pub fn is_empty(&self) -> bool {
        self.missing_headings.is_empty() && self.invalid_sections.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPlanPaths {
    pub plan_dir: PathBuf,
    pub state_path: PathBuf,
}

pub fn session_plan_paths(ctx: &ToolContext) -> Result<SessionPlanPaths> {
    let plan_dir = session_dir_for_tool_results(ctx)?.join(PLAN_DIR_NAME);
    Ok(SessionPlanPaths {
        state_path: plan_dir.join(PLAN_STATE_FILE_NAME),
        plan_dir,
    })
}

pub fn session_plan_markdown_path(plan_dir: &Path, slug: &str) -> PathBuf {
    plan_dir.join(format!("{slug}.md"))
}

pub fn workflow_state_path(ctx: &ToolContext) -> Result<PathBuf> {
    Ok(session_dir_for_tool_results(ctx)?
        .join(WORKFLOW_DIR_NAME)
        .join(WORKFLOW_STATE_FILE_NAME))
}

pub fn load_session_plan_state(path: &Path) -> Result<Option<SessionPlanState>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path)
        .map_err(|error| AstrError::io(format!("failed reading '{}'", path.display()), error))?;
    serde_json::from_str::<SessionPlanState>(&content)
        .map(Some)
        .map_err(|error| AstrError::parse("failed to parse session plan state", error))
}

pub fn persist_session_plan_state(path: &Path, state: &SessionPlanState) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Err(AstrError::Internal(format!(
            "session plan state '{}' has no parent directory",
            path.display()
        )));
    };
    fs::create_dir_all(parent).map_err(|error| {
        AstrError::io(
            format!(
                "failed creating session plan directory '{}'",
                parent.display()
            ),
            error,
        )
    })?;
    let content = serde_json::to_string_pretty(state)
        .map_err(|error| AstrError::parse("failed to serialize session plan state", error))?;
    fs::write(path, content).map_err(|error| {
        AstrError::io(
            format!("failed writing session plan state '{}'", path.display()),
            error,
        )
    })?;
    Ok(())
}

pub fn persist_planning_workflow_state(
    ctx: &ToolContext,
    plan_state: Option<&SessionPlanState>,
) -> Result<()> {
    let mut artifact_refs = BTreeMap::new();
    let plan_paths = session_plan_paths(ctx)?;
    if let Some(plan_state) = plan_state {
        if let Some(plan_artifact) = current_plan_artifact_ref(&plan_paths.plan_dir, plan_state) {
            artifact_refs.insert("canonical-plan".to_string(), plan_artifact);
        }
    }
    persist_workflow_state(
        &workflow_state_path(ctx)?,
        &WorkflowInstanceState {
            workflow_id: PLAN_EXECUTE_WORKFLOW_ID.to_string(),
            current_phase_id: PLANNING_PHASE_ID.to_string(),
            artifact_refs,
            bridge_state: None,
            updated_at: plan_state
                .map(|state| state.updated_at)
                .unwrap_or_else(Utc::now),
        },
    )
}

pub fn load_workflow_state(path: &Path) -> Result<Option<WorkflowInstanceState>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path)
        .map_err(|error| AstrError::io(format!("failed reading '{}'", path.display()), error))?;
    serde_json::from_str::<WorkflowInstanceState>(&content)
        .map(Some)
        .map_err(|error| AstrError::parse("failed to parse workflow state", error))
}

pub fn persist_workflow_state(path: &Path, state: &WorkflowInstanceState) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Err(AstrError::Internal(format!(
            "workflow state '{}' has no parent directory",
            path.display()
        )));
    };
    fs::create_dir_all(parent).map_err(|error| {
        AstrError::io(
            format!("failed creating workflow directory '{}'", parent.display()),
            error,
        )
    })?;
    let content = serde_json::to_string_pretty(state)
        .map_err(|error| AstrError::parse("failed to serialize workflow state", error))?;
    fs::write(path, content).map_err(|error| {
        AstrError::io(
            format!("failed writing workflow state '{}'", path.display()),
            error,
        )
    })?;
    Ok(())
}

pub fn validate_plan_artifact_contract(
    content: &str,
    artifact: &ModeArtifactDef,
) -> PlanArtifactContractBlockers {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return PlanArtifactContractBlockers {
            missing_headings: artifact
                .required_headings
                .iter()
                .map(|heading| markdown_section_heading(heading))
                .collect(),
            invalid_sections: Vec::new(),
        };
    }

    let missing_headings = artifact
        .required_headings
        .iter()
        .map(|heading| markdown_section_heading(heading))
        .filter(|heading| !trimmed.contains(heading))
        .collect::<Vec<_>>();

    let invalid_sections = artifact
        .actionable_sections
        .iter()
        .filter_map(|section| ensure_actionable_section(trimmed, section).err())
        .collect::<Vec<_>>();

    PlanArtifactContractBlockers {
        missing_headings,
        invalid_sections,
    }
}

fn current_plan_artifact_ref(
    plan_dir: &Path,
    plan_state: &SessionPlanState,
) -> Option<WorkflowArtifactRef> {
    let plan_path = session_plan_markdown_path(plan_dir, &plan_state.active_plan_slug);
    let Ok(content) = fs::read_to_string(&plan_path) else {
        return None;
    };
    Some(WorkflowArtifactRef {
        artifact_kind: "canonical-plan".to_string(),
        path: plan_path.display().to_string(),
        content_digest: Some(session_plan_content_digest(content.trim())),
    })
}

fn markdown_section_heading(heading: &str) -> String {
    let trimmed = heading.trim();
    if trimmed.starts_with('#') {
        trimmed.to_string()
    } else {
        format!("## {trimmed}")
    }
}

fn ensure_actionable_section(content: &str, heading: &str) -> std::result::Result<(), String> {
    let heading = markdown_section_heading(heading);
    let section = section_body(content, &heading)
        .ok_or_else(|| format!("session plan is missing required section '{}'", heading))?;
    let has_actionable_line = section.lines().map(str::trim).any(|line| {
        !line.is_empty()
            && (line.starts_with("- ")
                || line.starts_with("* ")
                || line.starts_with("+ ")
                || line.chars().next().is_some_and(|ch| ch.is_ascii_digit()))
    });
    if has_actionable_line {
        return Ok(());
    }
    Err(format!(
        "session plan section '{}' must contain concrete actionable items",
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

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn current_plan_artifact_ref_skips_missing_markdown_file() {
        let temp = tempdir().expect("tempdir should exist");
        let plan_state = SessionPlanState {
            active_plan_slug: "missing-plan".to_string(),
            title: "Missing Plan".to_string(),
            status: SessionPlanStatus::Draft,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            reviewed_plan_digest: None,
            approved_at: None,
            archived_plan_digest: None,
            archived_at: None,
        };

        let artifact = current_plan_artifact_ref(temp.path(), &plan_state);

        assert_eq!(artifact, None);
    }

    #[test]
    fn validate_plan_artifact_contract_uses_required_and_actionable_sections() {
        let blockers = validate_plan_artifact_contract(
            "# Plan\n\n## Context\n- grounded\n\n## Implementation Steps\nrefine later\n",
            &ModeArtifactDef {
                artifact_type: "canonical-plan".to_string(),
                file_template: None,
                schema_template: None,
                required_headings: vec![
                    "Context".to_string(),
                    "Implementation Steps".to_string(),
                    "Verification".to_string(),
                ],
                actionable_sections: vec![
                    "Implementation Steps".to_string(),
                    "Verification".to_string(),
                ],
            },
        );

        assert_eq!(
            blockers.missing_headings,
            vec!["## Verification".to_string()]
        );
        assert_eq!(blockers.invalid_sections.len(), 2);
    }
}
