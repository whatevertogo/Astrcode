//! # 四工具模型 — Observe 实现
//!
//! `observe` 是四工具模型（send / observe / close / interrupt）中的只读观察操作。
//! 从旧 runtime/service/agent/observe.rs 迁入，去掉对 RuntimeService 的依赖。
//!
//! 快照聚合两层：
//! 1. 从 kernel AgentControl 获取 lifecycle / turn_outcome
//! 2. 从 session-runtime 获取稳定 observe 视图

use astrcode_core::{
    AgentLifecycleStatus, CollaborationResult, CollaborationResultKind, ObserveAgentResult,
    ObserveParams,
};

use super::AgentOrchestrationService;

impl AgentOrchestrationService {
    /// 获取目标 child agent 的增强快照（四工具模型 observe）。
    pub async fn observe_child(
        &self,
        params: ObserveParams,
        ctx: &astrcode_core::ToolContext,
    ) -> Result<CollaborationResult, super::AgentOrchestrationError> {
        params
            .validate()
            .map_err(super::AgentOrchestrationError::from)?;

        let child = self
            .kernel
            .get_agent_handle(&params.agent_id)
            .await
            .ok_or_else(|| {
                super::AgentOrchestrationError::NotFound(format!(
                    "agent '{}' not found",
                    params.agent_id
                ))
            })?;

        self.verify_caller_owns_child(ctx, &child)?;

        let lifecycle_status = self
            .kernel
            .get_agent_lifecycle(&params.agent_id)
            .await
            .unwrap_or(AgentLifecycleStatus::Pending);

        let last_turn_outcome = self.kernel.get_agent_turn_outcome(&params.agent_id).await;

        let open_session_id = child
            .child_session_id
            .clone()
            .unwrap_or_else(|| child.session_id.clone());

        let observe_snapshot = self
            .session_runtime
            .observe_agent_session(&open_session_id, &params.agent_id, lifecycle_status)
            .await
            .map_err(|e| {
                super::AgentOrchestrationError::Internal(format!(
                    "failed to build observe snapshot: {e}"
                ))
            })?;

        let observe_result = ObserveAgentResult {
            agent_id: child.agent_id.clone(),
            sub_run_id: child.sub_run_id.clone(),
            session_id: child.session_id.clone(),
            open_session_id,
            parent_agent_id: child.parent_agent_id.clone().unwrap_or_default(),
            lifecycle_status,
            last_turn_outcome,
            phase: format!("{:?}", observe_snapshot.phase),
            turn_count: observe_snapshot.turn_count,
            pending_message_count: observe_snapshot.pending_message_count,
            active_task: observe_snapshot.active_task,
            pending_task: observe_snapshot.pending_task,
            last_output: observe_snapshot.last_output,
        };

        log::info!(
            "observe: snapshot for child agent '{}' (lifecycle={:?}, pending={})",
            params.agent_id,
            lifecycle_status,
            observe_result.pending_message_count
        );

        Ok(CollaborationResult {
            accepted: true,
            kind: CollaborationResultKind::Observed,
            agent_ref: Some(
                self.project_child_ref_status(self.build_child_ref_from_handle(&child).await)
                    .await,
            ),
            delivery_id: None,
            summary: Some(serde_json::to_string(&observe_result).unwrap_or_default()),
            cascade: None,
            closed_root_agent_id: None,
            failure: None,
        })
    }
}
