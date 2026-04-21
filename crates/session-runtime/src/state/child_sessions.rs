use std::collections::HashMap;

use astrcode_core::{ChildSessionNode, Result, StorageEventPayload, StoredEvent, support};

use super::SessionState;

pub(crate) fn rebuild_child_nodes(events: &[StoredEvent]) -> HashMap<String, ChildSessionNode> {
    let mut nodes = HashMap::new();
    for stored in events {
        if let Some(node) = child_node_from_stored_event(stored) {
            nodes.insert(node.sub_run_id().to_string(), node);
        }
    }
    nodes
}

pub(crate) fn child_node_from_stored_event(stored: &StoredEvent) -> Option<ChildSessionNode> {
    match &stored.event.payload {
        StorageEventPayload::ChildSessionNotification { notification, .. } => {
            Some(notification.child_ref.to_child_session_node(
                stored.event.turn_id.clone().unwrap_or_default().into(),
                astrcode_core::ChildSessionStatusSource::Durable,
                notification.source_tool_call_id.clone(),
                None,
            ))
        },
        _ => None,
    }
}

impl SessionState {
    /// 写入或覆盖一个 child-session durable 节点（按 sub_run_id 去重）。
    pub fn upsert_child_session_node(&self, node: ChildSessionNode) -> Result<()> {
        support::lock_anyhow(&self.projection_registry, "session projection registry")?
            .upsert_child_session_node(node);
        Ok(())
    }

    /// 查询某个 sub-run 对应的 child-session 节点快照。
    pub fn child_session_node(&self, sub_run_id: &str) -> Result<Option<ChildSessionNode>> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .child_session_node(sub_run_id),
        )
    }

    /// 列出当前 session 所有 child-session 节点快照（按 sub_run_id 排序）。
    pub fn list_child_session_nodes(&self) -> Result<Vec<ChildSessionNode>> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .list_child_session_nodes(),
        )
    }

    /// 查找某个 agent 的直接子节点。
    pub fn child_nodes_for_parent(&self, parent_agent_id: &str) -> Result<Vec<ChildSessionNode>> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .child_nodes_for_parent(parent_agent_id),
        )
    }

    /// 收集指定 agent 子树的所有后代节点（不含自身）。
    pub fn subtree_nodes(&self, root_agent_id: &str) -> Result<Vec<ChildSessionNode>> {
        Ok(
            support::lock_anyhow(&self.projection_registry, "session projection registry")?
                .subtree_nodes(root_agent_id),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use astrcode_core::{
        AgentLifecycleStatus, AgentStateProjector, ChildSessionNotificationKind, Phase,
    };

    use super::*;
    use crate::state::{
        SessionWriter,
        test_support::{NoopEventLogWriter, child_notification_event, stored},
    };

    #[test]
    fn session_state_rehydrates_child_nodes_from_stored_notifications() {
        let session = SessionState::new(
            Phase::Idle,
            Arc::new(SessionWriter::new(Box::new(NoopEventLogWriter))),
            AgentStateProjector::default(),
            Vec::new(),
            vec![
                stored(
                    1,
                    child_notification_event(
                        ChildSessionNotificationKind::Started,
                        AgentLifecycleStatus::Running,
                    ),
                ),
                stored(
                    2,
                    child_notification_event(
                        ChildSessionNotificationKind::Delivered,
                        AgentLifecycleStatus::Idle,
                    ),
                ),
            ],
        );

        let node = session
            .child_session_node("subrun-1")
            .expect("child node lookup should succeed")
            .expect("child node should exist");

        assert_eq!(node.child_session_id, "session-child".into());
        assert_eq!(node.parent_session_id, "session-parent".into());
        assert_eq!(node.status, AgentLifecycleStatus::Idle);
        assert_eq!(node.created_by_tool_call_id.as_deref(), Some("call-1"));
    }
}
