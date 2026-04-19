use crate::{
    App, ApplicationError,
    terminal::{
        ConversationFocus, TerminalFacts, TerminalRehydrateFacts, TerminalRehydrateReason,
        TerminalStreamFacts, TerminalStreamReplayFacts,
    },
};

impl App {
    pub async fn conversation_snapshot_facts(
        &self,
        session_id: &str,
        focus: ConversationFocus,
    ) -> Result<TerminalFacts, ApplicationError> {
        self.validate_non_empty("sessionId", session_id)?;
        let focus_session_id = self
            .resolve_conversation_focus_session_id(session_id, &focus)
            .await?;
        let transcript = self
            .session_runtime
            .conversation_snapshot(&focus_session_id)
            .await?;
        let session_title = self
            .session_runtime
            .list_session_metas()
            .await?
            .into_iter()
            .find(|meta| meta.session_id == session_id)
            .map(|meta| meta.title)
            .ok_or_else(|| {
                ApplicationError::NotFound(format!("session '{session_id}' not found"))
            })?;
        let control = self.terminal_control_facts(session_id).await?;
        let child_summaries = self
            .conversation_child_summaries(session_id, &focus)
            .await?;
        let slash_candidates = self.terminal_slash_candidates(session_id, None).await?;

        Ok(TerminalFacts {
            active_session_id: session_id.to_string(),
            session_title,
            transcript,
            control,
            child_summaries,
            slash_candidates,
        })
    }

    pub async fn terminal_snapshot_facts(
        &self,
        session_id: &str,
    ) -> Result<TerminalFacts, ApplicationError> {
        self.conversation_snapshot_facts(session_id, ConversationFocus::Root)
            .await
    }

    pub async fn conversation_stream_facts(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
        focus: ConversationFocus,
    ) -> Result<TerminalStreamFacts, ApplicationError> {
        self.validate_non_empty("sessionId", session_id)?;
        let focus_session_id = self
            .resolve_conversation_focus_session_id(session_id, &focus)
            .await?;

        if let Some(requested_cursor) = last_event_id {
            super::cursor::validate_cursor_format(requested_cursor)?;
            let transcript = self
                .session_runtime
                .conversation_snapshot(&focus_session_id)
                .await?;
            let latest_cursor = crate::terminal::latest_transcript_cursor(&transcript);
            if super::cursor::cursor_is_after_head(requested_cursor, latest_cursor.as_deref())? {
                return Ok(TerminalStreamFacts::RehydrateRequired(
                    TerminalRehydrateFacts {
                        session_id: session_id.to_string(),
                        requested_cursor: requested_cursor.to_string(),
                        latest_cursor,
                        reason: TerminalRehydrateReason::CursorExpired,
                    },
                ));
            }
        }

        let replay = self
            .session_runtime
            .conversation_stream_replay(&focus_session_id, last_event_id)
            .await?;
        let control = self.terminal_control_facts(session_id).await?;
        let child_summaries = self
            .conversation_child_summaries(session_id, &focus)
            .await?;
        let slash_candidates = self.terminal_slash_candidates(session_id, None).await?;

        Ok(TerminalStreamFacts::Replay(Box::new(
            TerminalStreamReplayFacts {
                active_session_id: session_id.to_string(),
                replay,
                control,
                child_summaries,
                slash_candidates,
            },
        )))
    }

    pub async fn terminal_stream_facts(
        &self,
        session_id: &str,
        last_event_id: Option<&str>,
    ) -> Result<TerminalStreamFacts, ApplicationError> {
        self.conversation_stream_facts(session_id, last_event_id, ConversationFocus::Root)
            .await
    }
}
