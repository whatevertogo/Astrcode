//! session 计划工件的共享读写辅助。
//!
//! `upsertSessionPlan`、`exitPlanMode` 等工具都需要读写同一份单 plan 状态；
//! 这里集中维护状态结构和路径规则，避免多处各自漂移。

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use astrcode_core::{
    AstrError, Result, ToolContext, WorkflowBridgeState, session_plan_content_digest,
};
pub use astrcode_core::{SessionPlanState, SessionPlanStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::builtin_tools::fs_common::session_dir_for_tool_results;

pub const PLAN_DIR_NAME: &str = "plan";
pub const PLAN_STATE_FILE_NAME: &str = "state.json";
pub const PLAN_PATH_TIMESTAMP_FORMAT: &str = "%Y%m%dT%H%M%SZ";
pub const WORKFLOW_DIR_NAME: &str = "workflow";
pub const WORKFLOW_STATE_FILE_NAME: &str = "state.json";
pub const PLAN_EXECUTE_WORKFLOW_ID: &str = "plan_execute";
pub const PLANNING_PHASE_ID: &str = "planning";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionPlanPaths {
    pub plan_dir: PathBuf,
    pub state_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowArtifactRef {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub artifact_kind: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowInstanceState {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub workflow_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub current_phase_id: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub artifact_refs: BTreeMap<String, WorkflowArtifactRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bridge_state: Option<WorkflowBridgeState>,
    pub updated_at: DateTime<Utc>,
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
}
