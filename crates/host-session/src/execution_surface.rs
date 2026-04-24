/// `host-session` 持有的最小快照骨架。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostSessionSnapshot {
    pub session_id: String,
    pub working_dir: String,
    pub event_log_revision: u64,
    pub read_model_revision: u64,
    pub lineage_parent_session_id: Option<String>,
    pub active_turn_id: Option<String>,
    pub mode_id: Option<String>,
}

impl HostSessionSnapshot {
    pub fn new(session_id: impl Into<String>, working_dir: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            working_dir: working_dir.into(),
            event_log_revision: 0,
            read_model_revision: 0,
            lineage_parent_session_id: None,
            active_turn_id: None,
            mode_id: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::HostSessionSnapshot;

    #[test]
    fn new_snapshot_starts_without_active_turn() {
        let snapshot = HostSessionSnapshot::new("session-1", "D:/repo");

        assert_eq!(snapshot.session_id, "session-1");
        assert_eq!(snapshot.working_dir, "D:/repo");
        assert_eq!(snapshot.event_log_revision, 0);
        assert!(snapshot.active_turn_id.is_none());
    }
}
