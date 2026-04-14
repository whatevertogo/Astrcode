use std::collections::HashMap;

use astrcode_core::{ChildSessionNode, Result, StorageEventPayload, StoredEvent, support};

use super::SessionState;

pub(crate) fn rebuild_child_nodes(events: &[StoredEvent]) -> HashMap<String, ChildSessionNode> {
    let mut nodes = HashMap::new();
    for stored in events {
        if let Some(node) = child_node_from_stored_event(stored) {
            nodes.insert(node.sub_run_id.clone(), node);
        }
    }
    nodes
}

pub(crate) fn child_node_from_stored_event(stored: &StoredEvent) -> Option<ChildSessionNode> {
    match &stored.event.payload {
        StorageEventPayload::ChildSessionNotification { notification, .. } => {
            Some(ChildSessionNode {
                agent_id: notification.child_ref.agent_id.clone(),
                session_id: notification.child_ref.session_id.clone(),
                child_session_id: notification.child_ref.open_session_id.clone(),
                sub_run_id: notification.child_ref.sub_run_id.clone(),
                parent_session_id: notification.child_ref.session_id.clone(),
                parent_agent_id: notification.child_ref.parent_agent_id.clone(),
                parent_sub_run_id: notification.child_ref.parent_sub_run_id.clone(),
                parent_turn_id: stored.event.turn_id.clone().unwrap_or_default(),
                lineage_kind: notification.child_ref.lineage_kind,
                status: notification.status,
                status_source: astrcode_core::ChildSessionStatusSource::Durable,
                created_by_tool_call_id: notification.source_tool_call_id.clone(),
                lineage_snapshot: None,
            })
        },
        _ => None,
    }
}

impl SessionState {
    /// 写入或覆盖一个 child-session durable 节点（按 sub_run_id 去重）。
    pub fn upsert_child_session_node(&self, node: ChildSessionNode) -> Result<()> {
        support::lock_anyhow(&self.child_nodes, "session child nodes")?
            .insert(node.sub_run_id.clone(), node);
        Ok(())
    }

    /// 查询某个 sub-run 对应的 child-session 节点快照。
    pub fn child_session_node(&self, sub_run_id: &str) -> Result<Option<ChildSessionNode>> {
        Ok(
            support::lock_anyhow(&self.child_nodes, "session child nodes")?
                .get(sub_run_id)
                .cloned(),
        )
    }

    /// 列出当前 session 所有 child-session 节点快照（按 sub_run_id 排序）。
    pub fn list_child_session_nodes(&self) -> Result<Vec<ChildSessionNode>> {
        let nodes = support::lock_anyhow(&self.child_nodes, "session child nodes")?;
        let mut result: Vec<_> = nodes.values().cloned().collect();
        result.sort_by(|a, b| a.sub_run_id.cmp(&b.sub_run_id));
        Ok(result)
    }

    /// 查找某个 agent 的直接子节点。
    pub fn child_nodes_for_parent(&self, parent_agent_id: &str) -> Result<Vec<ChildSessionNode>> {
        let nodes = support::lock_anyhow(&self.child_nodes, "session child nodes")?;
        let mut result: Vec<_> = nodes
            .values()
            .filter(|node| node.parent_agent_id.as_deref() == Some(parent_agent_id))
            .cloned()
            .collect();
        result.sort_by(|a, b| a.sub_run_id.cmp(&b.sub_run_id));
        Ok(result)
    }

    /// 收集指定 agent 子树的所有后代节点（不含自身）。
    pub fn subtree_nodes(&self, root_agent_id: &str) -> Result<Vec<ChildSessionNode>> {
        let nodes = support::lock_anyhow(&self.child_nodes, "session child nodes")?;
        let mut result = Vec::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(root_agent_id.to_string());
        while let Some(agent_id) = queue.pop_front() {
            for node in nodes.values() {
                if node.parent_agent_id.as_deref() == Some(&agent_id) {
                    queue.push_back(node.agent_id.clone());
                    result.push(node.clone());
                }
            }
        }
        result.sort_by(|a, b| a.sub_run_id.cmp(&b.sub_run_id));
        Ok(result)
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

        assert_eq!(node.child_session_id, "session-child");
        assert_eq!(node.parent_session_id, "session-parent");
        assert_eq!(node.status, AgentLifecycleStatus::Idle);
        assert_eq!(node.created_by_tool_call_id.as_deref(), Some("call-1"));
    }
}
