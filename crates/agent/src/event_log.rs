mod paths;
mod query;
mod store;

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;

use astrcode_core::Phase;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use self::paths::generate_session_id;
use self::paths::{session_path, validated_session_id};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub session_id: String,
    pub working_dir: String,
    pub display_name: String,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub phase: Phase,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DeleteProjectResult {
    pub success_count: usize,
    pub failed_session_ids: Vec<String>,
}

pub struct EventLog {
    session_id: String,
    path: PathBuf,
    writer: BufWriter<File>,
    next_storage_seq: u64,
}

impl Drop for EventLog {
    fn drop(&mut self) {
        if let Err(error) = self.writer.flush() {
            log::warn!(
                "failed to flush event log '{}' on drop: {}",
                self.path.display(),
                error
            );
            return;
        }

        if let Err(error) = self.writer.get_ref().sync_all() {
            log::warn!(
                "failed to sync event log '{}' on drop: {}",
                self.path.display(),
                error
            );
        }
    }
}

#[cfg(test)]
mod tests;
