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

    pub async fn cancel_subrun(&self, session_id: &str, sub_run_id: &str) -> ServiceResult<()> {
        let session_id = normalize_session_id(session_id);
        if let Some(handle) = self.runtime.agent_control.get(sub_run_id).await {
            if normalize_session_id(&handle.session_id) == session_id {
                let _ = self.runtime.agent_control.cancel(sub_run_id).await;
                return Ok(());
            }
        }

        let events = crate::service::session::load_events(
            Arc::clone(&self.runtime.session_manager),
            &session_id,
        )
        .await?;
        let Some(_snapshot) = find_subrun_status_in_events(&events, &session_id, sub_run_id) else {
            return Err(ServiceError::NotFound(format!(
                "sub-run '{}' was not found in session '{}'",
                sub_run_id, session_id
            )));
        };

        if self.runtime.agent_control.get(sub_run_id).await.is_some() {
            let _ = self.runtime.agent_control.cancel(sub_run_id).await;
        }

        // 已经结束的子会话视为幂等取消成功，避免前端在状态边缘切换时收到无意义错误。
        Ok(())
    }
}
