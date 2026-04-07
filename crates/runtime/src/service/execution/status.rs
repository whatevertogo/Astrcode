use std::sync::Arc;

use astrcode_runtime_execution::{
    CancelSubRunResolution, ParsedSubRunStatus, ParsedSubRunStatusSource,
    find_subrun_status_in_events, resolve_cancel_subrun_resolution, resolve_subrun_status_snapshot,
};
use astrcode_runtime_session::normalize_session_id;

use super::root::AgentExecutionServiceHandle;
use crate::service::{ServiceError, ServiceResult, SubRunStatusSnapshot, SubRunStatusSource};

impl AgentExecutionServiceHandle {
    pub async fn get_subrun_status(
        &self,
        session_id: &str,
        sub_run_id: &str,
    ) -> ServiceResult<SubRunStatusSnapshot> {
        let session_id = normalize_session_id(session_id);
        let events = crate::service::session::load_events(
            Arc::clone(&self.runtime.session_manager),
            &session_id,
        )
        .await?;
        let durable_snapshot = find_subrun_status_in_events(&events, &session_id, sub_run_id);
        let live_handle = self.runtime.agent_control.get(sub_run_id).await;
        let Some(snapshot) = resolve_subrun_status_snapshot(
            &session_id,
            live_handle,
            durable_snapshot,
            normalize_session_id,
        ) else {
            return Err(ServiceError::NotFound(format!(
                "sub-run '{}' was not found in session '{}'",
                sub_run_id, session_id
            )));
        };
        Ok(to_service_subrun_snapshot(snapshot))
    }

    pub async fn cancel_subrun(&self, session_id: &str, sub_run_id: &str) -> ServiceResult<()> {
        let session_id = normalize_session_id(session_id);
        let live_handle = self.runtime.agent_control.get(sub_run_id).await;

        let events = crate::service::session::load_events(
            Arc::clone(&self.runtime.session_manager),
            &session_id,
        )
        .await?;
        let durable_snapshot = find_subrun_status_in_events(&events, &session_id, sub_run_id);

        match resolve_cancel_subrun_resolution(
            &session_id,
            live_handle.as_ref(),
            durable_snapshot.as_ref(),
            normalize_session_id,
        ) {
            CancelSubRunResolution::CancelLive => {
                let _ = self.runtime.agent_control.cancel(sub_run_id).await;
                Ok(())
            },
            CancelSubRunResolution::AlreadyFinalized => {
                // 已经结束的子会话视为幂等取消成功，避免前端在状态边缘切换时收到无意义错误。
                Ok(())
            },
            CancelSubRunResolution::Missing => Err(ServiceError::NotFound(format!(
                "sub-run '{}' was not found in session '{}'",
                sub_run_id, session_id
            ))),
        }
    }
}

fn to_service_subrun_snapshot(snapshot: ParsedSubRunStatus) -> SubRunStatusSnapshot {
    SubRunStatusSnapshot {
        handle: snapshot.handle,
        descriptor: snapshot.descriptor,
        tool_call_id: snapshot.tool_call_id,
        source: map_subrun_status_source(snapshot.source),
        result: snapshot.result,
        step_count: snapshot.step_count,
        estimated_tokens: snapshot.estimated_tokens,
        resolved_overrides: snapshot.resolved_overrides,
        resolved_limits: snapshot.resolved_limits,
    }
}

fn map_subrun_status_source(source: ParsedSubRunStatusSource) -> SubRunStatusSource {
    match source {
        ParsedSubRunStatusSource::Live => SubRunStatusSource::Live,
        ParsedSubRunStatusSource::Durable => SubRunStatusSource::Durable,
        ParsedSubRunStatusSource::LegacyDurable => SubRunStatusSource::LegacyDurable,
    }
}
