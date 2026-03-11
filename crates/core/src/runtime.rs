mod assembly;
mod event_sink;

use std::collections::HashMap;

use anyhow::Result;
use chrono::Utc;
use tokio_util::sync::CancellationToken;

use crate::action::rebuild_reasoning_cache_from_events;
use crate::agent_loop::AgentLoop;
use crate::event_log::{generate_session_id, DeleteProjectResult, EventLog, SessionMeta};
use crate::events::StorageEvent;
use crate::llm::EventSink;
use crate::projection::{project, AgentState};

use self::assembly::build_agent_loop;
use self::event_sink::RuntimeEventSink;

pub struct AgentRuntime {
    pub session_id: String,
    log: EventLog,
    events_cache: Vec<StorageEvent>,
    loop_: AgentLoop,
}

impl AgentRuntime {
    /// Create a brand new session, writing a SessionStart event.
    pub fn new_session() -> Result<Self> {
        let session_id = generate_session_id();
        let mut log = EventLog::create(&session_id)?;

        let working_dir = std::env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        let session_start = StorageEvent::SessionStart {
            session_id: session_id.clone(),
            timestamp: Utc::now(),
            working_dir,
        };
        log.append(&session_start)?;

        let loop_ = build_agent_loop()?;

        Ok(Self {
            session_id,
            log,
            events_cache: vec![session_start],
            loop_,
        })
    }

    /// Resume an existing session.
    pub fn resume(session_id: &str) -> Result<Self> {
        let log = EventLog::open(session_id)?;
        let events_cache = EventLog::load_from_path(log.path())?;
        let loop_ = build_agent_loop()?;
        loop_.replace_reasoning_cache(rebuild_reasoning_cache_from_events(&events_cache));
        Ok(Self {
            session_id: session_id.to_string(),
            log,
            events_cache,
            loop_,
        })
    }

    /// Resume the last session (by alphabetical order), or None if no sessions exist.
    pub fn resume_last() -> Result<Option<Self>> {
        let sessions = EventLog::list_sessions()?;
        match sessions.last() {
            Some(id) => Ok(Some(Self::resume(id)?)),
            None => Ok(None),
        }
    }

    /// Get the current AgentState by replaying all events.
    pub fn state(&self) -> Result<AgentState> {
        if self.events_cache.is_empty() {
            let events = EventLog::load_from_path(self.log.path())?;
            return Ok(project(&events));
        }
        Ok(project(&self.events_cache))
    }

    /// Submit a user message: append it, rebuild state, run a turn, and emit+log
    /// every event produced by the agent loop.
    ///
    /// `cancel` is managed by the caller (Tauri handle) for interruption.
    /// `emit` is called for every StorageEvent so the caller can bridge to IPC.
    pub async fn submit(
        &mut self,
        text: String,
        cancel: CancellationToken,
        mut emit: impl FnMut(&StorageEvent),
    ) -> Result<()> {
        if self.events_cache.is_empty() {
            self.events_cache = EventLog::load_from_path(self.log.path())?;
            self.loop_
                .replace_reasoning_cache(rebuild_reasoning_cache_from_events(&self.events_cache));
        }

        let user_event = StorageEvent::UserMessage {
            content: text,
            timestamp: Utc::now(),
        };

        let loop_ = &self.loop_;
        let mut sink = RuntimeEventSink::new(
            &mut self.log,
            &mut self.events_cache,
            &mut emit,
            cancel.clone(),
        );
        sink.record_user_event(user_event)?;
        let state = project(sink.cached_events());

        loop_
            .run_turn(
                &state,
                &mut |event: StorageEvent| sink.handle_runtime_event(event),
                cancel,
            )
            .await?;

        sink.finish()
    }

    pub fn replace_reasoning_cache(&self, cache: HashMap<usize, String>) {
        self.loop_.replace_reasoning_cache(cache);
    }

    pub fn reasoning_cache_snapshot(&self) -> HashMap<usize, String> {
        self.loop_.reasoning_cache_snapshot()
    }

    pub fn set_transient_llm_sink(&self, sink: Option<EventSink>) {
        self.loop_.set_transient_llm_sink(sink);
    }

    pub fn list_sessions() -> Result<Vec<String>> {
        EventLog::list_sessions()
    }

    pub fn list_sessions_with_meta() -> Result<Vec<SessionMeta>> {
        EventLog::list_sessions_with_meta()
    }

    pub fn delete_session(session_id: &str) -> Result<()> {
        EventLog::delete_session(session_id)
    }

    pub fn delete_project(working_dir: &str) -> Result<DeleteProjectResult> {
        EventLog::delete_sessions_by_working_dir(working_dir)
    }
}

#[cfg(test)]
mod tests;
