use std::time::Duration;

use anyhow::Result;
use astrcode_client::{
    AstrcodeClientTransport, AstrcodeCompactSessionRequest, AstrcodeConversationBannerErrorCodeDto,
    AstrcodeConversationErrorEnvelopeDto, AstrcodeCreateSessionRequest,
    AstrcodeExecutionControlDto, AstrcodePromptRequest, ConversationStreamItem,
};

use super::{
    Action, AppController, filter_resume_sessions, required_working_dir, slash_query_from_input,
};
use crate::{
    command::{Command, InputAction, OverlayAction, classify_input, filter_slash_candidates},
    state::{OverlayState, StreamRenderMode},
};

impl<T> AppController<T>
where
    T: AstrcodeClientTransport + 'static,
{
    pub(super) async fn submit_current_input(&mut self) {
        let input = self.state.take_input();
        match classify_input(input.as_str()) {
            InputAction::Empty => {},
            InputAction::SubmitPrompt { text } => {
                let Some(session_id) = self.state.conversation.active_session_id.clone() else {
                    self.state.set_error_status("no active session");
                    return;
                };
                self.state.set_status("submitting prompt");
                let client = self.client.clone();
                let sender = self.actions_tx.clone();
                tokio::spawn(async move {
                    let result = client
                        .submit_prompt(
                            &session_id,
                            AstrcodePromptRequest {
                                text,
                                control: None,
                            },
                        )
                        .await;
                    let _ = sender.send(Action::PromptSubmitted { session_id, result });
                });
            },
            InputAction::RunCommand(command) => {
                self.execute_command(command).await;
            },
        }
    }

    pub(super) async fn execute_overlay_action(&mut self, action: OverlayAction) -> Result<()> {
        match action {
            OverlayAction::SwitchSession { session_id } => {
                self.state.close_overlay();
                self.begin_session_hydration(session_id).await;
            },
            OverlayAction::ReplaceInput { text } => {
                self.state.close_overlay();
                self.state.replace_input(text);
            },
            OverlayAction::RunCommand(command) => {
                self.state.close_overlay();
                self.execute_command(command).await;
            },
        }
        Ok(())
    }

    pub(super) async fn execute_command(&mut self, command: Command) {
        match command {
            Command::New => {
                let working_dir = match required_working_dir(&self.state) {
                    Ok(path) => path.display().to_string(),
                    Err(error) => {
                        self.state.set_error_status(error.to_string());
                        return;
                    },
                };
                let client = self.client.clone();
                let sender = self.actions_tx.clone();
                self.state.set_status("creating session");
                tokio::spawn(async move {
                    let result = client
                        .create_session(AstrcodeCreateSessionRequest { working_dir })
                        .await;
                    let _ = sender.send(Action::SessionCreated(result));
                });
            },
            Command::Resume { query } => {
                let query = query.unwrap_or_default();
                let items =
                    filter_resume_sessions(&self.state.conversation.sessions, query.as_str());
                self.state.set_resume_query(query, items);
                self.refresh_sessions().await;
            },
            Command::Compact => {
                let Some(session_id) = self.state.conversation.active_session_id.clone() else {
                    self.state.set_error_status("no active session");
                    return;
                };
                if self
                    .state
                    .conversation
                    .control
                    .as_ref()
                    .is_some_and(|control| !control.can_request_compact)
                {
                    self.state
                        .set_error_status("compact is not available right now");
                    return;
                }
                let client = self.client.clone();
                let sender = self.actions_tx.clone();
                self.state.set_status("requesting compact");
                tokio::spawn(async move {
                    let result = client
                        .request_compact(
                            &session_id,
                            AstrcodeCompactSessionRequest {
                                control: Some(AstrcodeExecutionControlDto {
                                    max_steps: None,
                                    manual_compact: Some(true),
                                }),
                                instructions: None,
                            },
                        )
                        .await;
                    let _ = sender.send(Action::CompactRequested { session_id, result });
                });
            },
            Command::Skill { query } => {
                self.open_slash_palette(query.unwrap_or_default()).await;
            },
            Command::Unknown { raw } => {
                self.state
                    .set_error_status(format!("unknown slash command: {raw}"));
            },
        }
    }

    pub(super) async fn begin_session_hydration(&mut self, session_id: String) {
        self.pending_session_id = Some(session_id.clone());
        if let Some(stream_task) = self.stream_task.take() {
            stream_task.abort();
        }
        self.stream_pacer.reset();
        self.state
            .set_status(format!("hydrating session {}", session_id));
        let client = self.client.clone();
        let sender = self.actions_tx.clone();
        tokio::spawn(async move {
            let result = client.fetch_conversation_snapshot(&session_id, None).await;
            let _ = sender.send(Action::SnapshotLoaded { session_id, result });
        });
    }

    pub(super) async fn open_stream_for_active_session(&mut self) {
        if let Some(stream_task) = self.stream_task.take() {
            stream_task.abort();
        }
        self.stream_pacer.reset();
        let Some(session_id) = self.state.conversation.active_session_id.clone() else {
            return;
        };
        let cursor = self.state.conversation.cursor.clone();
        match self
            .client
            .stream_conversation(&session_id, cursor.as_ref(), None)
            .await
        {
            Ok(mut stream) => {
                let sender = self.actions_tx.clone();
                let pacer = self.stream_pacer.clone();
                self.stream_task = Some(tokio::spawn(async move {
                    while let Ok(Some(item)) = stream.recv().await {
                        let mut items = vec![item];
                        if matches!(pacer.mode(), StreamRenderMode::CatchUp) {
                            while items.len() < 6 {
                                match tokio::time::timeout(Duration::from_millis(2), stream.recv())
                                    .await
                                {
                                    Ok(Ok(Some(next))) => items.push(next),
                                    _ => break,
                                }
                            }
                        }
                        pacer.note_enqueued(items.len());
                        if sender
                            .send(Action::StreamBatch {
                                session_id: session_id.clone(),
                                items,
                            })
                            .is_err()
                        {
                            break;
                        }
                    }
                }));
            },
            Err(error) => self.apply_banner_error(error),
        }
    }

    pub(super) async fn refresh_sessions(&self) {
        let client = self.client.clone();
        let sender = self.actions_tx.clone();
        tokio::spawn(async move {
            let result = client.list_sessions().await;
            let _ = sender.send(Action::SessionsRefreshed(result));
        });
    }

    pub(super) async fn open_slash_palette(&mut self, query: String) {
        let items = if query.trim().is_empty() {
            self.state.conversation.slash_candidates.clone()
        } else {
            filter_slash_candidates(&self.state.conversation.slash_candidates, &query)
        };
        self.state.set_slash_query(query.clone(), items);
        self.refresh_slash_candidates(query).await;
    }

    pub(super) async fn refresh_slash_candidates(&self, query: String) {
        let Some(session_id) = self.state.conversation.active_session_id.clone() else {
            return;
        };
        let client = self.client.clone();
        let sender = self.actions_tx.clone();
        tokio::spawn(async move {
            let result = client
                .list_conversation_slash_candidates(&session_id, Some(query.as_str()))
                .await;
            let _ = sender.send(Action::SlashCandidatesLoaded { query, result });
        });
    }

    pub(super) async fn refresh_overlay_query(&mut self) {
        match &self.state.interaction.overlay {
            OverlayState::Resume(resume) => {
                let items = filter_resume_sessions(
                    &self.state.conversation.sessions,
                    resume.query.as_str(),
                );
                self.state.set_resume_query(resume.query.clone(), items);
            },
            OverlayState::SlashPalette(palette) => {
                self.refresh_slash_candidates(palette.query.clone()).await;
            },
            OverlayState::DebugLogs(_) => {},
            OverlayState::None => {},
        }
    }

    pub(super) fn refresh_resume_overlay(&mut self) {
        let OverlayState::Resume(resume) = &self.state.interaction.overlay else {
            return;
        };
        let items =
            filter_resume_sessions(&self.state.conversation.sessions, resume.query.as_str());
        self.state.set_resume_query(resume.query.clone(), items);
    }

    pub(super) async fn apply_stream_event(
        &mut self,
        session_id: &str,
        item: ConversationStreamItem,
    ) {
        match item {
            ConversationStreamItem::Delta(envelope) => {
                self.state.clear_banner();
                self.state.apply_stream_envelope(*envelope);
            },
            ConversationStreamItem::RehydrateRequired(error) => {
                self.state.set_banner_error(error);
                self.begin_session_hydration(session_id.to_string()).await;
            },
            ConversationStreamItem::Lagged { skipped } => {
                self.state
                    .set_banner_error(AstrcodeConversationErrorEnvelopeDto {
                        code: AstrcodeConversationBannerErrorCodeDto::CursorExpired,
                        message: format!("stream lagged by {skipped} events, rehydrating"),
                        rehydrate_required: true,
                        details: None,
                    });
                self.begin_session_hydration(session_id.to_string()).await;
            },
            ConversationStreamItem::Disconnected { message } => {
                self.state
                    .set_banner_error(AstrcodeConversationErrorEnvelopeDto {
                        code: AstrcodeConversationBannerErrorCodeDto::StreamDisconnected,
                        message,
                        rehydrate_required: false,
                        details: None,
                    });
            },
        }
    }

    pub(super) fn slash_query_for_current_input(&self) -> String {
        slash_query_from_input(self.state.interaction.composer.input.as_str())
    }
}
