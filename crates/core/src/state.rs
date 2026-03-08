#[derive(Debug, Clone)]
pub struct SessionState {
    pub session_id: String,
    pub active_turn_id: Option<String>,
}

impl SessionState {
    pub fn new(session_id: String) -> Self {
        Self {
            session_id,
            active_turn_id: None,
        }
    }
}
