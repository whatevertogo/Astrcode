//! session 计划工件的共享读写辅助。
//!
//! `upsertSessionPlan`、`exitPlanMode` 等工具都需要读写同一份单 plan 状态；
//! 这里集中维护状态结构和路径规则，避免多处各自漂移。

use std::{
    fs,
    path::{Path, PathBuf},
};

use astrcode_core::{AstrError, Result, ToolContext};
pub use astrcode_core::{SessionPlanState, SessionPlanStatus};

use crate::builtin_tools::fs_common::session_dir_for_tool_results;

pub const PLAN_DIR_NAME: &str = "plan";
pub const PLAN_STATE_FILE_NAME: &str = "state.json";
pub const PLAN_PATH_TIMESTAMP_FORMAT: &str = "%Y%m%dT%H%M%SZ";

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
