//! observe 快照聚合。

use astrcode_core::{
    AgentLifecycleStatus, CollaborationResult, CollaborationResultKind, ObserveAgentResult,
    ObserveParams,
};

use super::AgentServiceHandle;
use crate::service::{ServiceError, ServiceResult};

impl AgentServiceHandle {
    /// 获取目标 child agent 的增强快照（四工具模型 observe）。
    pub async fn observe_child(
        &self,
        params: ObserveParams,
        ctx: &astrcode_core::ToolContext,
    ) -> ServiceResult<CollaborationResult> {
        params.validate().map_err(ServiceError::from)?;

        let child = self
            .runtime
            .agent_control
            .get(&params.agent_id)
            .await
            .ok_or_else(|| {
                ServiceError::NotFound(format!("agent '{}' not found", params.agent_id))
            })?;

        self.verify_caller_owns_child(ctx, &child)?;

        let lifecycle_status = self
            .runtime
            .agent_control
            .get_lifecycle(&params.agent_id)
            .await
            .unwrap_or(AgentLifecycleStatus::Pending);

        let last_turn_outcome = self
            .runtime
            .agent_control
            .get_turn_outcome(&params.agent_id)
            .await
            .flatten();

        let open_session_id = child
            .child_session_id
            .clone()
            .unwrap_or_else(|| child.session_id.clone());
        let session_state = self.runtime.ensure_session_loaded(&open_session_id).await?;
        let projected = session_state.snapshot_projected_state().map_err(|error| {
            ServiceError::Internal(astrcode_core::AstrError::Internal(error.to_string()))
        })?;

        let pending_message_count = session_state
            .mailbox_projection_for_agent(&params.agent_id)
            .map(|p| p.pending_message_count())
            .unwrap_or(0);

        let observe_result = ObserveAgentResult {
            agent_id: child.agent_id.clone(),
            sub_run_id: child.sub_run_id.clone(),
            session_id: child.session_id.clone(),
            open_session_id,
            parent_agent_id: child.parent_agent_id.clone().unwrap_or_default(),
            lifecycle_status,
            last_turn_outcome,
            phase: format!("{:?}", projected.phase),
            turn_count: projected.turn_count as u32,
            pending_message_count,
            last_output: extract_last_output(&projected.messages),
        };

        log::info!(
            "observe: snapshot for child agent '{}' (lifecycle={:?}, pending={})",
            params.agent_id,
            lifecycle_status,
            pending_message_count
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

/// 从消息列表中提取最后一条 assistant 消息的输出摘要。
pub(super) fn extract_last_output(messages: &[astrcode_core::LlmMessage]) -> Option<String> {
    messages.iter().rev().find_map(|msg| match msg {
        astrcode_core::LlmMessage::Assistant { content, .. } => {
            if content.is_empty() {
                None
            } else if content.len() > 200 {
                Some(format!("{}...", &content[..200]))
            } else {
                Some(content.clone())
            }
        },
        _ => None,
    })
}
