use std::{
    fs,
    path::{Path, PathBuf},
};

use astrcode_core::WorkflowInstanceState;
use astrcode_support::hostpaths::project_dir;

use crate::ApplicationError;

const WORKFLOW_DIR_NAME: &str = "workflow";
const WORKFLOW_STATE_FILE_NAME: &str = "state.json";

#[derive(Debug, Clone, Default)]
pub struct WorkflowStateService;

impl WorkflowStateService {
    pub fn workflow_dir(session_id: &str, working_dir: &Path) -> Result<PathBuf, ApplicationError> {
        Ok(project_dir(working_dir)
            .map_err(|error| {
                ApplicationError::Internal(format!(
                    "failed to resolve project directory for '{}': {error}",
                    working_dir.display()
                ))
            })?
            .join("sessions")
            .join(session_id)
            .join(WORKFLOW_DIR_NAME))
    }

    pub fn state_path(session_id: &str, working_dir: &Path) -> Result<PathBuf, ApplicationError> {
        Ok(Self::workflow_dir(session_id, working_dir)?.join(WORKFLOW_STATE_FILE_NAME))
    }

    pub fn load(
        session_id: &str,
        working_dir: &Path,
    ) -> Result<Option<WorkflowInstanceState>, ApplicationError> {
        let path = Self::state_path(session_id, working_dir)?;
        if !path.exists() {
            return Ok(None);
        }
        let content =
            fs::read_to_string(&path).map_err(|error| io_error("reading", &path, error))?;
        serde_json::from_str::<WorkflowInstanceState>(&content)
            .map(Some)
            .map_err(|error| {
                ApplicationError::Internal(format!(
                    "failed to parse workflow state '{}': {error}",
                    path.display()
                ))
            })
    }

    pub fn load_recovering(
        session_id: &str,
        working_dir: &Path,
    ) -> Result<Option<WorkflowInstanceState>, ApplicationError> {
        let path = Self::state_path(session_id, working_dir)?;
        match Self::load(session_id, working_dir) {
            Ok(state) => Ok(state),
            Err(error) => {
                log::warn!(
                    "failed to recover workflow state '{}', degrading to mode-only: {}",
                    path.display(),
                    error
                );
                Ok(None)
            },
        }
    }

    pub fn persist(
        session_id: &str,
        working_dir: &Path,
        state: &WorkflowInstanceState,
    ) -> Result<(), ApplicationError> {
        let path = Self::state_path(session_id, working_dir)?;
        let Some(parent) = path.parent() else {
            return Err(ApplicationError::Internal(format!(
                "workflow state '{}' has no parent directory",
                path.display()
            )));
        };
        fs::create_dir_all(parent)
            .map_err(|error| io_error("creating directory", parent, error))?;
        let content = serde_json::to_string_pretty(state).map_err(|error| {
            ApplicationError::Internal(format!(
                "failed to serialize workflow state '{}': {error}",
                path.display()
            ))
        })?;
        fs::write(&path, content).map_err(|error| io_error("writing", &path, error))
    }

    pub fn clear(session_id: &str, working_dir: &Path) -> Result<(), ApplicationError> {
        let path = Self::state_path(session_id, working_dir)?;
        if !path.exists() {
            return Ok(());
        }
        fs::remove_file(&path).map_err(|error| io_error("removing", &path, error))
    }
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> ApplicationError {
    ApplicationError::Internal(format!("{action} '{}' failed: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs};

    use astrcode_core::{WorkflowArtifactRef, WorkflowInstanceState};
    use chrono::{TimeZone, Utc};
    use tempfile::tempdir;

    use super::WorkflowStateService;

    #[test]
    fn workflow_state_service_round_trips_state_file() {
        let _guard = astrcode_core::test_support::TestEnvGuard::new();
        let temp = tempdir().expect("tempdir should exist");
        let state = WorkflowInstanceState {
            workflow_id: "plan_execute".to_string(),
            current_phase_id: "planning".to_string(),
            artifact_refs: BTreeMap::from([(
                "canonical-plan".to_string(),
                WorkflowArtifactRef {
                    artifact_kind: "canonical-plan".to_string(),
                    path: "/tmp/plan.md".to_string(),
                    content_digest: Some("abc".to_string()),
                },
            )]),
            bridge_state: None,
            updated_at: Utc
                .with_ymd_and_hms(2026, 4, 21, 9, 0, 0)
                .single()
                .expect("datetime should be valid"),
        };

        WorkflowStateService::persist("session-a", temp.path(), &state)
            .expect("state should persist");
        let loaded = WorkflowStateService::load("session-a", temp.path())
            .expect("state should load")
            .expect("state should exist");

        assert_eq!(loaded, state);
        assert!(
            WorkflowStateService::state_path("session-a", temp.path())
                .expect("path should resolve")
                .display()
                .to_string()
                .ends_with("workflow\\state.json")
                || WorkflowStateService::state_path("session-a", temp.path())
                    .expect("path should resolve")
                    .display()
                    .to_string()
                    .ends_with("workflow/state.json")
        );
    }

    #[test]
    fn load_recovering_downgrades_invalid_json_to_none() {
        let _guard = astrcode_core::test_support::TestEnvGuard::new();
        let temp = tempdir().expect("tempdir should exist");
        let path = WorkflowStateService::state_path("session-a", temp.path())
            .expect("path should resolve");
        fs::create_dir_all(path.parent().expect("parent should exist"))
            .expect("parent dir should exist");
        fs::write(&path, "{not-json").expect("invalid state should be written");

        let loaded = WorkflowStateService::load_recovering("session-a", temp.path())
            .expect("recovery should not fail");
        assert!(loaded.is_none());
    }
}
