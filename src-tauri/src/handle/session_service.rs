use std::collections::HashSet;

use astrcode_core::{AgentRuntime, DeleteProjectResult, EventLog, SessionMeta};

use super::presentation::convert_events_to_messages;
use super::support::{
    canonical_session_id, same_working_dir, sync_runtime_working_dir, user_home_dir,
};
use super::{AgentHandle, SessionMessage};

impl AgentHandle {
    pub async fn get_session_id(&self) -> String {
        canonical_session_id(&self.session_id.lock().await).to_string()
    }

    pub fn list_sessions() -> Result<Vec<String>, String> {
        AgentRuntime::list_sessions().map_err(|e| e.to_string())
    }

    pub fn list_sessions_with_meta() -> Result<Vec<SessionMeta>, String> {
        AgentRuntime::list_sessions_with_meta().map_err(|e| e.to_string())
    }

    pub fn load_session(session_id: &str) -> Result<Vec<SessionMessage>, String> {
        let session_id = canonical_session_id(session_id);
        let events = EventLog::load(session_id).map_err(|e| e.to_string())?;
        Ok(convert_events_to_messages(&events))
    }

    pub async fn new_session(&self) -> Result<String, String> {
        self.interrupt().await?;

        let runtime = AgentRuntime::new_session().map_err(|e| e.to_string())?;
        let session_id = runtime.session_id.clone();
        let session_cache = runtime.reasoning_cache_snapshot();

        *self.runtime.lock().await = runtime;
        *self.session_id.lock().await = session_id.clone();
        self.reasoning_cache
            .lock()
            .await
            .insert(session_id.clone(), session_cache);

        Ok(session_id)
    }

    pub async fn switch_session(&self, session_id: &str) -> Result<(), String> {
        let session_id = canonical_session_id(session_id);

        self.interrupt().await?;

        let runtime = AgentRuntime::resume(session_id).map_err(|e| e.to_string())?;
        sync_runtime_working_dir(&runtime);
        let session_cache = runtime.reasoning_cache_snapshot();

        *self.runtime.lock().await = runtime;
        *self.session_id.lock().await = session_id.to_string();
        self.reasoning_cache
            .lock()
            .await
            .insert(session_id.to_string(), session_cache);

        Ok(())
    }

    pub async fn delete_session(&self, session_id: String) -> Result<(), String> {
        let target_id = canonical_session_id(&session_id).to_string();
        let current_id = canonical_session_id(&self.session_id.lock().await).to_string();

        if current_id == target_id {
            self.interrupt().await?;
            let runtime = AgentRuntime::new_session().map_err(|e| e.to_string())?;
            let next_session_id = runtime.session_id.clone();
            let session_cache = runtime.reasoning_cache_snapshot();

            sync_runtime_working_dir(&runtime);

            *self.runtime.lock().await = runtime;
            *self.session_id.lock().await = next_session_id.clone();
            self.reasoning_cache
                .lock()
                .await
                .insert(next_session_id, session_cache);
        }

        self.reasoning_cache.lock().await.remove(&target_id);
        AgentRuntime::delete_session(&target_id).map_err(|e| e.to_string())
    }

    pub async fn delete_project(&self, working_dir: String) -> Result<DeleteProjectResult, String> {
        let metas = AgentRuntime::list_sessions_with_meta().map_err(|e| e.to_string())?;
        let targets: HashSet<String> = metas
            .iter()
            .filter(|meta| same_working_dir(&meta.working_dir, &working_dir))
            .map(|meta| meta.session_id.clone())
            .collect();

        if targets.is_empty() {
            return Ok(DeleteProjectResult {
                success_count: 0,
                failed_session_ids: Vec::new(),
            });
        }

        let current_id = canonical_session_id(&self.session_id.lock().await).to_string();
        if targets.contains(&current_id) {
            self.interrupt().await?;

            if let Some(replacement) = metas
                .iter()
                .find(|meta| !targets.contains(&meta.session_id))
            {
                let runtime =
                    AgentRuntime::resume(&replacement.session_id).map_err(|e| e.to_string())?;
                sync_runtime_working_dir(&runtime);
                let session_cache = runtime.reasoning_cache_snapshot();
                *self.runtime.lock().await = runtime;
                *self.session_id.lock().await = replacement.session_id.clone();
                self.reasoning_cache
                    .lock()
                    .await
                    .insert(replacement.session_id.clone(), session_cache);
            } else {
                let home = user_home_dir()
                    .ok_or_else(|| "unable to resolve home directory".to_string())?;
                std::env::set_current_dir(&home).map_err(|e| e.to_string())?;
                let runtime = AgentRuntime::new_session().map_err(|e| e.to_string())?;
                let session_id = runtime.session_id.clone();
                let session_cache = runtime.reasoning_cache_snapshot();
                *self.runtime.lock().await = runtime;
                *self.session_id.lock().await = session_id.clone();
                self.reasoning_cache
                    .lock()
                    .await
                    .insert(session_id, session_cache);
            }
        }

        self.reasoning_cache
            .lock()
            .await
            .retain(|session_id, _| !targets.contains(session_id));
        AgentRuntime::delete_project(&working_dir).map_err(|e| e.to_string())
    }

    pub async fn interrupt(&self) -> Result<(), String> {
        let mut guard = self.cancel.lock().await;
        if let Some(token) = guard.take() {
            token.cancel();
        }
        Ok(())
    }
}
