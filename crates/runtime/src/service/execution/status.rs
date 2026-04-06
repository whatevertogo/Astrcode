use std::sync::Arc;

use astrcode_runtime_execution::{find_subrun_status_in_events, snapshot_from_active_handle};
use astrcode_runtime_session::normalize_session_id;

use super::root::AgentExecutionServiceHandle;
use crate::service::{ServiceError, ServiceResult, SubRunStatusSnapshot};

impl AgentExecutionServiceHandle {
    pub async fn get_subrun_status(
        &self,
        session_id: &str,
        sub_run_id: &str,
    ) -> ServiceResult<SubRunStatusSnapshot> {
        let session_id = normalize_session_id(session_id);
        if let Some(handle) = self.runtime.agent_control.get(sub_run_id).await {
            let snapshot = snapshot_from_active_handle(handle);
            return Ok(SubRunStatusSnapshot {
                handle: snapshot.handle,
                result: snapshot.result,
                step_count: snapshot.step_count,
                estimated_tokens: snapshot.estimated_tokens,
                resolved_overrides: snapshot.resolved_overrides,
                resolved_limits: snapshot.resolved_limits,
            });
        }
        let events = crate::service::session::load_events(
            Arc::clone(&self.runtime.session_manager),
            &session_id,
        )
        .await?;
        let Some(snapshot) = find_subrun_status_in_events(&events, &session_id, sub_run_id) else {
            return Err(ServiceError::NotFound(format!(
                "sub-run '{}' was not found in session '{}'",
                sub_run_id, session_id
            )));
        };
        Ok(SubRunStatusSnapshot {
            handle: snapshot.handle,
            result: snapshot.result,
            step_count: snapshot.step_count,
            estimated_tokens: snapshot.estimated_tokens,
            resolved_overrides: snapshot.resolved_overrides,
            resolved_limits: snapshot.resolved_limits,
        })
    }
}
