use std::sync::Arc;

use astrcode_core::{
    AstrError, SessionTurnAcquireResult, SessionTurnLease, StorageEvent, StoredEvent,
    generate_session_id,
};
use astrcode_runtime_session::SessionState;
use chrono::Utc;

use crate::service::{
    RuntimeService, ServiceError, ServiceResult, SessionCatalogEvent,
    blocking_bridge::spawn_blocking_service,
};

/// Turn 提交真正落地的目标，包括分支后 session 和独占 turn lease。
pub(super) struct SubmitTarget {
    pub(super) session_id: String,
    pub(super) branched_from_session_id: Option<String>,
    pub(super) session: Arc<SessionState>,
    pub(super) turn_lease: Box<dyn SessionTurnLease>,
}

/// 并发分支深度不应该无限增长，否则说明调用侧持续对忙会话进行并发写入。
const MAX_CONCURRENT_BRANCH_DEPTH: usize = 3;

impl RuntimeService {
    pub(super) async fn resolve_submit_target(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> ServiceResult<SubmitTarget> {
        let mut target_session_id = session_id.to_string();
        let mut branched_from_session_id = None;
        let mut branch_depth = 0usize;

        loop {
            let session = self.ensure_session_loaded(&target_session_id).await?;
            let session_manager = Arc::clone(&self.session_manager);
            let acquire_session_id = target_session_id.clone();
            let acquire_turn_id = turn_id.to_string();
            let acquire_result = spawn_blocking_service("acquire session turn lease", move || {
                session_manager
                    .try_acquire_turn(&acquire_session_id, &acquire_turn_id)
                    .map_err(ServiceError::from)
            })
            .await?;

            match acquire_result {
                SessionTurnAcquireResult::Acquired(turn_lease) => {
                    return Ok(SubmitTarget {
                        session_id: target_session_id,
                        branched_from_session_id,
                        session,
                        turn_lease,
                    });
                },
                SessionTurnAcquireResult::Busy(active_turn) => {
                    ensure_branch_depth_within_limit(branch_depth)?;
                    let source_session_id = target_session_id.clone();
                    target_session_id = self
                        .branch_session_from_busy_turn(&source_session_id, &active_turn.turn_id)
                        .await?;
                    self.emit_session_catalog_event(SessionCatalogEvent::SessionBranched {
                        session_id: target_session_id.clone(),
                        source_session_id: source_session_id.clone(),
                    });
                    branched_from_session_id = Some(source_session_id);
                    branch_depth += 1;
                },
            }
        }
    }

    pub(super) async fn branch_session_from_busy_turn(
        &self,
        source_session_id: &str,
        active_turn_id: &str,
    ) -> ServiceResult<String> {
        let session_manager = Arc::clone(&self.session_manager);
        let source_session_id = source_session_id.to_string();
        let active_turn_id = active_turn_id.to_string();
        spawn_blocking_service("branch running session", move || {
            let source_events = session_manager
                .replay_events(&source_session_id)
                .map_err(ServiceError::from)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(ServiceError::from)?;
            let Some(first_event) = source_events.first() else {
                return Err(ServiceError::NotFound(format!(
                    "session '{}' is empty",
                    source_session_id
                )));
            };
            let working_dir = match &first_event.event {
                StorageEvent::SessionStart { working_dir, .. } => {
                    std::path::PathBuf::from(working_dir)
                },
                _ => {
                    return Err(ServiceError::Internal(AstrError::Internal(format!(
                        "session '{}' is missing sessionStart",
                        source_session_id
                    ))));
                },
            };

            let stable_events = stable_events_before_active_turn(&source_events, &active_turn_id);
            let parent_storage_seq = stable_events.last().map(|event| event.storage_seq);
            let branched_session_id = generate_session_id();
            let mut log = session_manager
                .create_event_log(&branched_session_id, &working_dir)
                .map_err(ServiceError::from)?;
            log.append(&StorageEvent::SessionStart {
                session_id: branched_session_id.clone(),
                timestamp: Utc::now(),
                working_dir: working_dir.to_string_lossy().to_string(),
                parent_session_id: Some(source_session_id.clone()),
                parent_storage_seq,
            })
            .map_err(ServiceError::from)?;

            // 分叉只复制已稳定完成的历史，避免把活跃 turn 的半截输出带入新分支。
            for stored in stable_events {
                if matches!(stored.event, StorageEvent::SessionStart { .. }) {
                    continue;
                }
                log.append(&stored.event).map_err(ServiceError::from)?;
            }

            Ok(branched_session_id)
        })
        .await
    }
}

pub(super) fn stable_events_before_active_turn(
    events: &[StoredEvent],
    active_turn_id: &str,
) -> Vec<StoredEvent> {
    let cutoff = events
        .iter()
        .position(|stored| stored.event.turn_id() == Some(active_turn_id))
        .unwrap_or(events.len());
    events[..cutoff].to_vec()
}

pub(super) fn ensure_branch_depth_within_limit(branch_depth: usize) -> ServiceResult<()> {
    if branch_depth >= MAX_CONCURRENT_BRANCH_DEPTH {
        return Err(ServiceError::Conflict(format!(
            "too many concurrent branch attempts (limit: {})",
            MAX_CONCURRENT_BRANCH_DEPTH
        )));
    }
    Ok(())
}
