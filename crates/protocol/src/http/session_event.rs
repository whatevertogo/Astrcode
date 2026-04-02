use serde::{Deserialize, Serialize};

use crate::http::PROTOCOL_VERSION;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum SessionCatalogEventPayload {
    SessionCreated {
        session_id: String,
    },
    SessionDeleted {
        session_id: String,
    },
    ProjectDeleted {
        working_dir: String,
    },
    SessionBranched {
        session_id: String,
        source_session_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionCatalogEventEnvelope {
    pub protocol_version: u32,
    #[serde(flatten)]
    pub event: SessionCatalogEventPayload,
}

impl SessionCatalogEventEnvelope {
    pub fn new(event: SessionCatalogEventPayload) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            event,
        }
    }
}
