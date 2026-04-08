//! Turn 中断：取消正在执行的 turn 及其子运行。

use astrcode_runtime_execution::resolve_interrupt_session_plan;

use crate::service::{
    ServiceResult, blocking_bridge::lock_anyhow, execution::AgentExecutionServiceHandle,
};

impl AgentExecutionServiceHandle {
    /// 中断指定会话的活跃 turn。
    ///
    /// 如果会话正在运行，取消其 cancel token 并传播到子运行时。
    /// 如果会话空闲或已结束，幂等返回成功。
    pub async fn interrupt_session(&self, session_id: &str) -> ServiceResult<()> {
        let session_id = astrcode_runtime_session::normalize_session_id(session_id);
        if let Some(session) = self.runtime.sessions.get(&session_id) {
            let is_running = session.running.load(std::sync::atomic::Ordering::SeqCst);
            let active_turn_id =
                lock_anyhow(&session.active_turn_id, "session active turn").map(|g| g.clone())?;
            let interrupt_plan =
                resolve_interrupt_session_plan(is_running, active_turn_id.as_deref());
            if !interrupt_plan.should_cancel_session {
                return Ok(());
            }
            if let Ok(cancel) = lock_anyhow(&session.cancel, "session cancel") {
                cancel.cancel();
            }
            if let Some(active_turn_id) = interrupt_plan.active_turn_id.as_deref() {
                // 故意忽略：取消子运行时失败不应阻断中断流程
                let _ = self
                    .runtime
                    .agent_control
                    .cancel_for_parent_turn(active_turn_id)
                    .await;
            }
        }
        Ok(())
    }
}
