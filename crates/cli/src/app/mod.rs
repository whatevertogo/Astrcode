use std::{
    env, io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use astrcode_client::{
    AstrcodeClient, AstrcodeClientError, AstrcodeClientErrorKind, AstrcodeCompactSessionRequest,
    AstrcodeConversationBannerErrorCodeDto, AstrcodeConversationErrorEnvelopeDto,
    AstrcodeConversationSlashCandidatesResponseDto, AstrcodeCreateSessionRequest,
    AstrcodeExecutionControlDto, AstrcodePromptRequest, AstrcodeSessionListItem, ClientConfig,
    ConversationStreamItem,
};
use clap::Parser;
use crossterm::{
    event::{self, Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::{
    sync::mpsc,
    task::JoinHandle,
    time::{self, MissedTickBehavior},
};

use crate::{
    command::{Command, InputAction, OverlayAction, classify_input, overlay_action},
    launcher::{LaunchOptions, Launcher, LauncherSession, SystemManagedServer},
    render,
    state::{CliState, OverlayState},
};

#[derive(Debug, Parser)]
#[command(name = "astrcode-cli")]
#[command(about = "Astrcode 的正式 terminal frontend")]
struct CliArgs {
    #[arg(long)]
    server_origin: Option<String>,
    #[arg(long)]
    token: Option<String>,
    #[arg(long)]
    working_dir: Option<PathBuf>,
    #[arg(long)]
    run_info_path: Option<PathBuf>,
    #[arg(long)]
    server_binary: Option<PathBuf>,
}

#[derive(Debug)]
enum Action {
    Tick,
    Key(KeyEvent),
    Quit,
    SessionsRefreshed(Result<Vec<AstrcodeSessionListItem>, AstrcodeClientError>),
    SessionCreated(Result<AstrcodeSessionListItem, AstrcodeClientError>),
    SnapshotLoaded {
        session_id: String,
        result: Result<astrcode_client::AstrcodeTerminalSnapshotResponseDto, AstrcodeClientError>,
    },
    StreamEvent {
        session_id: String,
        item: ConversationStreamItem,
    },
    SlashCandidatesLoaded {
        query: String,
        result: Result<AstrcodeConversationSlashCandidatesResponseDto, AstrcodeClientError>,
    },
    PromptSubmitted {
        session_id: String,
        result: Result<astrcode_client::AstrcodePromptAcceptedResponse, AstrcodeClientError>,
    },
    CompactRequested {
        session_id: String,
        result: Result<astrcode_client::AstrcodeCompactSessionResponse, AstrcodeClientError>,
    },
}

pub async fn run_from_env() -> Result<()> {
    let args = CliArgs::parse();
    let launcher = Launcher::new();
    let working_dir = resolve_working_dir(args.working_dir)?;
    let launch_options = LaunchOptions {
        server_origin: args.server_origin,
        bootstrap_token: args.token,
        working_dir: Some(working_dir.clone()),
        run_info_path: args.run_info_path,
        server_binary: args.server_binary,
        ..LaunchOptions::default()
    };
    let launcher_session = launcher.resolve(launch_options).await?;
    run_app(launcher_session).await
}

async fn run_app(launcher_session: LauncherSession<SystemManagedServer>) -> Result<()> {
    let connection = launcher_session.connection().clone();
    let client = AstrcodeClient::new(ClientConfig::new(connection.origin.clone()));
    client
        .exchange_auth(connection.bootstrap_token.clone())
        .await
        .context("exchange auth with astrcode-server failed")?;

    let (actions_tx, actions_rx) = mpsc::unbounded_channel();
    let mut controller = AppController::new(
        client,
        CliState::new(connection.origin, connection.working_dir.clone()),
        actions_tx.clone(),
        actions_rx,
    );

    controller.bootstrap().await?;

    let terminal_result = run_terminal_loop(&mut controller, actions_tx.clone()).await;

    controller.stop_background_tasks();
    let shutdown_result = launcher_session.shutdown().await;

    terminal_result?;
    shutdown_result?;
    Ok(())
}

async fn run_terminal_loop(
    controller: &mut AppController,
    actions_tx: mpsc::UnboundedSender<Action>,
) -> Result<()> {
    enable_raw_mode().context("enable raw mode failed")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("enter alternate screen failed")?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("create terminal backend failed")?;

    let input_handle = InputHandle::spawn(actions_tx.clone());
    let tick_handle = spawn_tick_loop(actions_tx);

    let loop_result = run_event_loop(controller, &mut terminal).await;

    input_handle.stop();
    tick_handle.abort();

    disable_raw_mode().context("disable raw mode failed")?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("leave alternate screen failed")?;
    terminal.show_cursor().context("show cursor failed")?;

    loop_result
}

async fn run_event_loop(
    controller: &mut AppController,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<()> {
    terminal
        .draw(|frame| render::render(frame, &controller.state))
        .context("initial draw failed")?;

    while let Some(action) = controller.actions_rx.recv().await {
        controller.handle_action(action).await?;
        terminal
            .draw(|frame| render::render(frame, &controller.state))
            .context("redraw failed")?;
        if controller.should_quit {
            break;
        }
    }

    Ok(())
}

struct AppController {
    client: AstrcodeClient,
    state: CliState,
    actions_tx: mpsc::UnboundedSender<Action>,
    actions_rx: mpsc::UnboundedReceiver<Action>,
    pending_session_id: Option<String>,
    stream_task: Option<JoinHandle<()>>,
    should_quit: bool,
}

impl AppController {
    fn new(
        client: AstrcodeClient,
        state: CliState,
        actions_tx: mpsc::UnboundedSender<Action>,
        actions_rx: mpsc::UnboundedReceiver<Action>,
    ) -> Self {
        Self {
            client,
            state,
            actions_tx,
            actions_rx,
            pending_session_id: None,
            stream_task: None,
            should_quit: false,
        }
    }

    async fn bootstrap(&mut self) -> Result<()> {
        self.refresh_sessions().await;
        let sessions = self
            .client
            .list_sessions()
            .await
            .context("load sessions during bootstrap failed")?;
        self.state.update_sessions(sessions.clone());

        let session_id = if let Some(session) =
            choose_initial_session(&sessions, self.state.working_dir.as_deref())
        {
            session.session_id.clone()
        } else {
            let created = self
                .client
                .create_session(AstrcodeCreateSessionRequest {
                    working_dir: required_working_dir(&self.state)?.display().to_string(),
                })
                .await
                .context("create initial session failed")?;
            self.state.update_sessions(vec![created.clone()]);
            created.session_id
        };

        self.begin_session_hydration(session_id).await;
        Ok(())
    }

    fn stop_background_tasks(&mut self) {
        if let Some(stream_task) = self.stream_task.take() {
            stream_task.abort();
        }
    }

    async fn handle_action(&mut self, action: Action) -> Result<()> {
        match action {
            Action::Tick => {},
            Action::Quit => self.should_quit = true,
            Action::Key(key) => self.handle_key(key).await?,
            Action::SessionsRefreshed(result) => match result {
                Ok(sessions) => {
                    self.state.update_sessions(sessions);
                    self.refresh_resume_overlay();
                },
                Err(error) => self.apply_status_error(error),
            },
            Action::SessionCreated(result) => match result {
                Ok(session) => {
                    let session_id = session.session_id.clone();
                    let mut sessions = self.state.sessions.clone();
                    sessions.push(session);
                    sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
                    self.state.update_sessions(sessions);
                    self.begin_session_hydration(session_id).await;
                },
                Err(error) => self.apply_status_error(error),
            },
            Action::SnapshotLoaded { session_id, result } => {
                if self.pending_session_id.as_deref() != Some(session_id.as_str()) {
                    return Ok(());
                }
                match result {
                    Ok(snapshot) => {
                        self.pending_session_id = None;
                        self.state.activate_snapshot(snapshot);
                        self.state
                            .set_status(format!("attached to session {}", session_id));
                        self.open_stream_for_active_session().await;
                    },
                    Err(error) => {
                        self.pending_session_id = None;
                        self.apply_hydration_error(error);
                    },
                }
            },
            Action::StreamEvent { session_id, item } => {
                if self.state.active_session_id.as_deref() != Some(session_id.as_str()) {
                    return Ok(());
                }
                match item {
                    ConversationStreamItem::Delta(envelope) => {
                        self.state.clear_banner();
                        self.state.apply_stream_envelope(*envelope);
                    },
                    ConversationStreamItem::RehydrateRequired(error) => {
                        self.state.set_banner_error(error);
                        self.begin_session_hydration(session_id).await;
                    },
                    ConversationStreamItem::Lagged { skipped } => {
                        self.state
                            .set_banner_error(AstrcodeConversationErrorEnvelopeDto {
                                code: AstrcodeConversationBannerErrorCodeDto::CursorExpired,
                                message: format!("stream lagged by {skipped} events, rehydrating"),
                                rehydrate_required: true,
                                details: None,
                            });
                        self.begin_session_hydration(session_id).await;
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
            },
            Action::SlashCandidatesLoaded { query, result } => {
                let OverlayState::SlashPalette(palette) = &self.state.overlay else {
                    return Ok(());
                };
                if palette.query != query {
                    return Ok(());
                }

                match result {
                    Ok(candidates) => {
                        self.state.set_slash_query(query, candidates.items);
                    },
                    Err(error) => self.apply_status_error(error),
                }
            },
            Action::PromptSubmitted { session_id, result } => {
                if self.state.active_session_id.as_deref() != Some(session_id.as_str()) {
                    return Ok(());
                }
                match result {
                    Ok(response) => {
                        self.state
                            .set_status(format!("prompt accepted: turn {}", response.turn_id));
                    },
                    Err(error) => self.apply_status_error(error),
                }
            },
            Action::CompactRequested { session_id, result } => {
                if self.state.active_session_id.as_deref() != Some(session_id.as_str()) {
                    return Ok(());
                }
                match result {
                    Ok(response) => {
                        self.state.set_status(response.message);
                    },
                    Err(error) => self.apply_status_error(error),
                }
            },
        }
        Ok(())
    }

    async fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
            return Ok(());
        }

        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('q'))
        {
            self.should_quit = true;
            return Ok(());
        }

        match key.code {
            KeyCode::Esc => self.state.close_overlay(),
            KeyCode::Up => {
                if matches!(self.state.overlay, OverlayState::None) {
                    self.state.scroll_up();
                } else {
                    self.state.overlay_prev();
                }
            },
            KeyCode::Down => {
                if matches!(self.state.overlay, OverlayState::None) {
                    self.state.scroll_down();
                } else {
                    self.state.overlay_next();
                }
            },
            KeyCode::Enter => {
                if let Some(selection) = self.state.selected_overlay() {
                    self.execute_overlay_action(overlay_action(selection))
                        .await?;
                } else {
                    self.submit_current_input().await;
                }
            },
            KeyCode::Backspace => {
                if matches!(self.state.overlay, OverlayState::None) {
                    self.state.pop_input();
                } else {
                    self.state.overlay_query_pop();
                    self.refresh_overlay_query().await;
                }
            },
            KeyCode::Tab => {
                let query = slash_query_from_input(self.state.composer.input.as_str());
                self.open_slash_palette(query).await;
            },
            KeyCode::Char(ch) => {
                if matches!(self.state.overlay, OverlayState::None) {
                    self.state.push_input(ch);
                } else {
                    self.state.overlay_query_push(ch);
                    self.refresh_overlay_query().await;
                }
            },
            _ => {},
        }

        Ok(())
    }

    async fn submit_current_input(&mut self) {
        let input = self.state.take_input();
        match classify_input(input.as_str()) {
            InputAction::Empty => {},
            InputAction::SubmitPrompt { text } => {
                let Some(session_id) = self.state.active_session_id.clone() else {
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

    async fn execute_overlay_action(&mut self, action: OverlayAction) -> Result<()> {
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

    async fn execute_command(&mut self, command: Command) {
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
                let items = filter_resume_sessions(&self.state.sessions, query.as_str());
                self.state.set_resume_query(query, items);
                self.refresh_sessions().await;
            },
            Command::Compact => {
                let Some(session_id) = self.state.active_session_id.clone() else {
                    self.state.set_error_status("no active session");
                    return;
                };
                if self
                    .state
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

    async fn begin_session_hydration(&mut self, session_id: String) {
        self.pending_session_id = Some(session_id.clone());
        if let Some(stream_task) = self.stream_task.take() {
            stream_task.abort();
        }
        self.state
            .set_status(format!("hydrating session {}", session_id));
        let client = self.client.clone();
        let sender = self.actions_tx.clone();
        tokio::spawn(async move {
            let result = client.fetch_conversation_snapshot(&session_id, None).await;
            let _ = sender.send(Action::SnapshotLoaded { session_id, result });
        });
    }

    async fn open_stream_for_active_session(&mut self) {
        if let Some(stream_task) = self.stream_task.take() {
            stream_task.abort();
        }
        let Some(session_id) = self.state.active_session_id.clone() else {
            return;
        };
        let cursor = self.state.cursor.clone();
        match self
            .client
            .stream_conversation(&session_id, cursor.as_ref(), None)
            .await
        {
            Ok(mut stream) => {
                let sender = self.actions_tx.clone();
                self.stream_task = Some(tokio::spawn(async move {
                    while let Ok(Some(item)) = stream.recv().await {
                        if sender
                            .send(Action::StreamEvent {
                                session_id: session_id.clone(),
                                item,
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

    async fn refresh_sessions(&self) {
        let client = self.client.clone();
        let sender = self.actions_tx.clone();
        tokio::spawn(async move {
            let result = client.list_sessions().await;
            let _ = sender.send(Action::SessionsRefreshed(result));
        });
    }

    async fn open_slash_palette(&mut self, query: String) {
        let items = if query.trim().is_empty() {
            self.state.slash_candidates.clone()
        } else {
            crate::command::filter_slash_candidates(&self.state.slash_candidates, &query)
        };
        self.state.set_slash_query(query.clone(), items);
        self.refresh_slash_candidates(query).await;
    }

    async fn refresh_slash_candidates(&self, query: String) {
        let Some(session_id) = self.state.active_session_id.clone() else {
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

    async fn refresh_overlay_query(&mut self) {
        match &self.state.overlay {
            OverlayState::Resume(resume) => {
                let items = filter_resume_sessions(&self.state.sessions, resume.query.as_str());
                self.state.set_resume_query(resume.query.clone(), items);
            },
            OverlayState::SlashPalette(palette) => {
                self.refresh_slash_candidates(palette.query.clone()).await;
            },
            OverlayState::None => {},
        }
    }

    fn refresh_resume_overlay(&mut self) {
        let OverlayState::Resume(resume) = &self.state.overlay else {
            return;
        };
        let items = filter_resume_sessions(&self.state.sessions, resume.query.as_str());
        self.state.set_resume_query(resume.query.clone(), items);
    }

    fn apply_status_error(&mut self, error: AstrcodeClientError) {
        self.state.set_error_status(error.message);
    }

    fn apply_hydration_error(&mut self, error: AstrcodeClientError) {
        match error.kind {
            AstrcodeClientErrorKind::AuthExpired
            | AstrcodeClientErrorKind::CursorExpired
            | AstrcodeClientErrorKind::StreamDisconnected
            | AstrcodeClientErrorKind::TransportUnavailable
            | AstrcodeClientErrorKind::UnexpectedResponse => self.apply_banner_error(error),
            _ => self.apply_status_error(error),
        }
    }

    fn apply_banner_error(&mut self, error: AstrcodeClientError) {
        self.state
            .set_banner_error(AstrcodeConversationErrorEnvelopeDto {
                code: match error.kind {
                    AstrcodeClientErrorKind::AuthExpired => {
                        AstrcodeConversationBannerErrorCodeDto::AuthExpired
                    },
                    AstrcodeClientErrorKind::CursorExpired => {
                        AstrcodeConversationBannerErrorCodeDto::CursorExpired
                    },
                    AstrcodeClientErrorKind::StreamDisconnected
                    | AstrcodeClientErrorKind::TransportUnavailable
                    | AstrcodeClientErrorKind::PermissionDenied
                    | AstrcodeClientErrorKind::Validation
                    | AstrcodeClientErrorKind::NotFound
                    | AstrcodeClientErrorKind::Conflict
                    | AstrcodeClientErrorKind::UnexpectedResponse => {
                        AstrcodeConversationBannerErrorCodeDto::StreamDisconnected
                    },
                },
                message: error.message.clone(),
                rehydrate_required: matches!(error.kind, AstrcodeClientErrorKind::CursorExpired),
                details: error.details,
            });
        self.state.set_error_status(error.message);
    }
}

struct InputHandle {
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}

impl InputHandle {
    fn spawn(actions_tx: mpsc::UnboundedSender<Action>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let join = thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                if event::poll(Duration::from_millis(100)).unwrap_or(false) {
                    match event::read() {
                        Ok(CrosstermEvent::Key(key)) => {
                            if actions_tx.send(Action::Key(key)).is_err() {
                                break;
                            }
                        },
                        Ok(_) => {},
                        Err(_) => {
                            let _ = actions_tx.send(Action::Quit);
                            break;
                        },
                    }
                }
            }
        });

        Self {
            stop,
            join: Some(join),
        }
    }

    fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

fn spawn_tick_loop(actions_tx: mpsc::UnboundedSender<Action>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = time::interval(Duration::from_millis(250));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if actions_tx.send(Action::Tick).is_err() {
                break;
            }
        }
    })
}

fn resolve_working_dir(cli_value: Option<PathBuf>) -> Result<PathBuf> {
    match cli_value {
        Some(path) => Ok(path),
        None => env::current_dir().context("resolve current working directory failed"),
    }
}

fn required_working_dir(state: &CliState) -> Result<&Path> {
    state
        .working_dir
        .as_deref()
        .context("working directory is required for /new")
}

fn choose_initial_session<'a>(
    sessions: &'a [AstrcodeSessionListItem],
    working_dir: Option<&Path>,
) -> Option<&'a AstrcodeSessionListItem> {
    let working_dir = working_dir.map(|path| path.display().to_string());
    sessions.iter().max_by(|left, right| {
        let left_matches = working_dir
            .as_ref()
            .is_some_and(|working_dir| left.working_dir == *working_dir);
        let right_matches = working_dir
            .as_ref()
            .is_some_and(|working_dir| right.working_dir == *working_dir);

        left_matches
            .cmp(&right_matches)
            .then_with(|| left.updated_at.cmp(&right.updated_at))
    })
}

fn filter_resume_sessions(
    sessions: &[AstrcodeSessionListItem],
    query: &str,
) -> Vec<AstrcodeSessionListItem> {
    let query = query.trim().to_lowercase();
    let mut items = sessions
        .iter()
        .filter(|session| {
            if query.is_empty() {
                return true;
            }
            session.session_id.to_lowercase().contains(&query)
                || session.title.to_lowercase().contains(&query)
                || session.display_name.to_lowercase().contains(&query)
                || session.working_dir.to_lowercase().contains(&query)
        })
        .cloned()
        .collect::<Vec<_>>();
    items.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    items
}

fn slash_query_from_input(input: &str) -> String {
    let trimmed = input.trim();
    if let Some(query) = trimmed.strip_prefix("/skill") {
        return query.trim().to_string();
    }
    trimmed.trim_start_matches('/').trim().to_string()
}

#[cfg(test)]
mod tests {
    use astrcode_client::AstrcodePhaseDto;

    use super::*;

    fn session(
        session_id: &str,
        working_dir: &str,
        title: &str,
        updated_at: &str,
    ) -> AstrcodeSessionListItem {
        AstrcodeSessionListItem {
            session_id: session_id.to_string(),
            working_dir: working_dir.to_string(),
            display_name: title.to_string(),
            title: title.to_string(),
            created_at: updated_at.to_string(),
            updated_at: updated_at.to_string(),
            parent_session_id: None,
            parent_storage_seq: None,
            phase: AstrcodePhaseDto::Idle,
        }
    }

    #[test]
    fn chooses_most_recent_session_in_same_working_dir() {
        let sessions = vec![
            session("s1", "D:/repo-a", "older", "2026-04-15T10:00:00Z"),
            session("s2", "D:/repo-b", "other", "2026-04-15T12:00:00Z"),
            session("s3", "D:/repo-a", "newer", "2026-04-15T11:00:00Z"),
        ];

        let selected =
            choose_initial_session(&sessions, Some(Path::new("D:/repo-a"))).expect("session");
        assert_eq!(selected.session_id, "s3");
    }

    #[test]
    fn resume_filter_matches_title_and_working_dir() {
        let sessions = vec![
            session(
                "s1",
                "D:/repo-a",
                "terminal-read-model",
                "2026-04-15T10:00:00Z",
            ),
            session("s2", "D:/other", "other", "2026-04-15T12:00:00Z"),
        ];

        assert_eq!(filter_resume_sessions(&sessions, "terminal").len(), 1);
        assert_eq!(filter_resume_sessions(&sessions, "repo-a").len(), 1);
    }
}
