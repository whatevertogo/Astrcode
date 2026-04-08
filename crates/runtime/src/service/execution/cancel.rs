//! Sub-run 取消控制：取消指定子运行。

use std::sync::Arc;

use astrcode_runtime_execution::{
    CancelSubRunResolution, find_subrun_status_in_events, resolve_cancel_subrun_resolution,
};
use astrcode_runtime_session::normalize_session_id;

use super::root::AgentExecutionServiceHandle;
use crate::service::{ServiceError, ServiceResult};

impl AgentExecutionServiceHandle {
    /// 取消指定 sub-run。
    ///
    /// 根据 live handle 和 durable 事件的快照决定取消策略：
    /// - `CancelLive`：向 live control plane 发送取消
    /// - `AlreadyFinalized`：幂等成功
    /// - `Missing`：返回 NotFound 错误
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
                // 故意忽略：取消子运行时失败不应阻断状态更新
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
