//! Sub-run 取消控制：取消指定子运行。

use std::sync::Arc;

use astrcode_core::{
    AgentStatus, DeleteProjectResult, StorageEvent, StorageEventPayload, ToolEventSink,
};
use astrcode_runtime_execution::{
    CancelSubRunResolution, build_child_session_notification, find_subrun_status_in_events,
    resolve_cancel_subrun_resolution,
};
use astrcode_runtime_session::normalize_session_id;

use super::root::AgentExecutionServiceHandle;
use crate::service::{ServiceError, ServiceResult};

impl AgentExecutionServiceHandle {
    pub async fn delete_session(&self, session_id: &str) -> ServiceResult<()> {
        let session_id = normalize_session_id(session_id);
        self.interrupt_session(&session_id).await?;
        self.runtime
            .sessions()
            .purge_session_durable(&session_id)
            .await
    }

    pub async fn delete_project(&self, working_dir: &str) -> ServiceResult<DeleteProjectResult> {
        let session_manager = Arc::clone(&self.runtime.session_manager);
        let working_dir_owned = working_dir.to_string();
        let metas = crate::service::blocking_bridge::spawn_blocking_service(
            "list project sessions before execution delete",
            move || {
                session_manager
                    .list_sessions_with_meta()
                    .map_err(ServiceError::from)
            },
        )
        .await?;
        let targets = metas
            .into_iter()
            .filter(|meta| meta.working_dir == working_dir_owned)
            .map(|meta| meta.session_id)
            .collect::<Vec<_>>();

        for session_id in &targets {
            let _ = self.interrupt_session(session_id).await;
        }

        self.runtime
            .sessions()
            .purge_project_durable(working_dir)
            .await
    }

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

    /// 按 agent 所有权子树执行级联关闭，而非按 parent turn 关闭。
    ///
    /// 与 `cancel_subrun` 不同，此方法：
    /// 1. 先收集子树中所有后代的 sub_run_id
    /// 2. 按 leaf-first 顺序取消（先深后浅）
    /// 3. 为每个被关闭的后代持久化 ChildSessionNotification(Closed)
    ///
    /// 这确保关闭传播遵循 agent ownership tree 语义，
    /// 而不是依赖 parent_turn_id 的隐式分组。
    pub async fn close_agent_subtree(
        &self,
        session_id: &str,
        agent_id: &str,
        cascade: bool,
    ) -> ServiceResult<Vec<String>> {
        let session_id = normalize_session_id(session_id);
        let session_state = self.runtime.ensure_session_loaded(&session_id).await?;

        if !cascade {
            // 非 cascade 模式只关闭目标 agent 自身
            self.runtime.agent_control.cancel(agent_id).await;
            self.emit_closed_notification(&session_state, agent_id)
                .await;
            return Ok(vec![agent_id.to_string()]);
        }

        // 收集子树中所有后代（不含自身）
        let subtree = self
            .runtime
            .agent_control
            .collect_subtree_handles(agent_id)
            .await;

        // 按 leaf-first 顺序关闭：depth 大的先取消
        let mut sorted_subtree = subtree;
        sorted_subtree.sort_by_key(|b| std::cmp::Reverse(b.depth));

        let mut closed_ids = Vec::new();

        // 先关闭所有后代
        for handle in &sorted_subtree {
            let _ = self.runtime.agent_control.cancel(&handle.agent_id).await;
            self.emit_closed_notification(&session_state, &handle.agent_id)
                .await;
            closed_ids.push(handle.agent_id.clone());
        }

        // 最后关闭目标 agent 自身
        let _ = self.runtime.agent_control.cancel(agent_id).await;
        self.emit_closed_notification(&session_state, agent_id)
            .await;
        closed_ids.push(agent_id.to_string());

        Ok(closed_ids)
    }

    /// 为被关闭的 agent 持久化 ChildSessionNotification(Closed) 通知。
    async fn emit_closed_notification(
        &self,
        session_state: &Arc<astrcode_runtime_session::SessionState>,
        agent_id: &str,
    ) {
        // 查找 durable 节点以构建通知
        let handle = match self.runtime.agent_control.get(agent_id).await {
            Some(h) => h,
            None => return,
        };

        // 通过 sub_run_id 查找 durable child_node
        let child_node = session_state
            .child_session_node(&handle.sub_run_id)
            .unwrap_or(None)
            .unwrap_or_else(|| {
                // 若 durable 节点不存在，构建一个临时节点
                astrcode_core::ChildSessionNode {
                    agent_id: handle.agent_id.clone(),
                    session_id: handle.session_id.clone(),
                    child_session_id: handle
                        .child_session_id
                        .clone()
                        .unwrap_or_else(|| handle.session_id.clone()),
                    sub_run_id: handle.sub_run_id.clone(),
                    parent_session_id: String::new(),
                    parent_agent_id: handle.parent_agent_id.clone(),
                    parent_turn_id: handle.parent_turn_id.clone(),
                    lineage_kind: astrcode_core::ChildSessionLineageKind::Spawn,
                    status: AgentStatus::Cancelled,
                    status_source: astrcode_core::ChildSessionStatusSource::Live,
                    created_by_tool_call_id: None,
                    lineage_snapshot: None,
                }
            });

        let notification = build_child_session_notification(
            &child_node,
            format!("child-closed:{}", handle.sub_run_id),
            astrcode_core::ChildSessionNotificationKind::Closed,
            format!("子 Agent {} 已被关闭。", handle.agent_id),
            AgentStatus::Cancelled,
            None,
        );

        let event_sink =
            match astrcode_runtime_session::SessionStateEventSink::new(Arc::clone(session_state)) {
                Ok(sink) => sink,
                Err(_) => return,
            };
        // 构建关闭事件
        let closed_event = StorageEvent {
            turn_id: Some(handle.parent_turn_id.clone()),
            agent: astrcode_core::AgentEventContext::sub_run(
                handle.agent_id.clone(),
                handle.parent_turn_id.clone(),
                handle.agent_profile.clone(),
                handle.sub_run_id.clone(),
                handle.storage_mode,
                handle.child_session_id.clone(),
            ),
            payload: StorageEventPayload::ChildSessionNotification {
                notification,
                timestamp: Some(chrono::Utc::now()),
            },
        };
        // 故意忽略：关闭通知持久化失败不应阻断级联关闭流程
        let _ = event_sink.emit(closed_event);
    }
}
