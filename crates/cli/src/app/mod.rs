mod coordinator;
mod reducer;

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
    AstrcodeClient, AstrcodeClientError, AstrcodeClientTransport,
    AstrcodeConversationSlashCandidatesResponseDto, AstrcodeConversationSnapshotResponseDto,
    AstrcodePromptAcceptedResponse, AstrcodeReqwestTransport, AstrcodeSessionListItem,
    ClientConfig, ConversationStreamItem,
};
use clap::Parser;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    },
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
    capability::TerminalCapabilities,
    command::overlay_action,
    launcher::{LaunchOptions, Launcher, LauncherSession, SystemManagedServer},
    render,
    state::{CliState, OverlayState, PaneFocus, StreamRenderMode},
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
    Paste(String),
    Resize {
        width: u16,
        height: u16,
    },
    Quit,
    SessionsRefreshed(Result<Vec<AstrcodeSessionListItem>, AstrcodeClientError>),
    SessionCreated(Result<AstrcodeSessionListItem, AstrcodeClientError>),
    SnapshotLoaded {
        session_id: String,
        result: Result<AstrcodeConversationSnapshotResponseDto, AstrcodeClientError>,
    },
    StreamBatch {
        session_id: String,
        items: Vec<ConversationStreamItem>,
    },
    SlashCandidatesLoaded {
        query: String,
        result: Result<AstrcodeConversationSlashCandidatesResponseDto, AstrcodeClientError>,
    },
    PromptSubmitted {
        session_id: String,
        result: Result<AstrcodePromptAcceptedResponse, AstrcodeClientError>,
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
    let mut launcher_session = launcher_session;
    let connection = launcher_session.connection().clone();
    let debug_tap = launcher_session
        .managed_server_mut()
        .map(|server| server.debug_tap());
    let client = AstrcodeClient::new(ClientConfig::new(connection.origin.clone()));
    let capabilities = TerminalCapabilities::detect();
    client
        .exchange_auth(connection.bootstrap_token.clone())
        .await
        .context("exchange auth with astrcode-server failed")?;

    let (actions_tx, actions_rx) = mpsc::unbounded_channel();
    let mut controller = AppController::new(
        client,
        CliState::new(
            connection.origin,
            connection.working_dir.clone(),
            capabilities,
        ),
        debug_tap,
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
    if controller.state.shell.capabilities.alt_screen {
        execute!(stdout, EnterAlternateScreen).context("enter alternate screen failed")?;
    }
    if controller.state.shell.capabilities.mouse {
        execute!(stdout, EnableMouseCapture).context("enable mouse capture failed")?;
    }
    if controller.state.shell.capabilities.bracketed_paste {
        execute!(stdout, EnableBracketedPaste).context("enable bracketed paste failed")?;
    }

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("create terminal backend failed")?;

    let input_handle = InputHandle::spawn(actions_tx.clone());
    let tick_handle = spawn_tick_loop(actions_tx);

    let loop_result = run_event_loop(controller, &mut terminal).await;

    input_handle.stop();
    tick_handle.abort();

    disable_raw_mode().context("disable raw mode failed")?;
    if controller.state.shell.capabilities.bracketed_paste {
        execute!(terminal.backend_mut(), DisableBracketedPaste)
            .context("disable bracketed paste failed")?;
    }
    if controller.state.shell.capabilities.mouse {
        execute!(terminal.backend_mut(), DisableMouseCapture)
            .context("disable mouse capture failed")?;
    }
    if controller.state.shell.capabilities.alt_screen {
        execute!(terminal.backend_mut(), LeaveAlternateScreen)
            .context("leave alternate screen failed")?;
    }
    terminal.show_cursor().context("show cursor failed")?;

    loop_result
}

async fn run_event_loop(
    controller: &mut AppController,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> Result<()> {
    terminal
        .draw(|frame| render::render(frame, &mut controller.state))
        .context("initial draw failed")?;

    while let Some(action) = controller.actions_rx.recv().await {
        controller.handle_action(action).await?;
        terminal
            .draw(|frame| render::render(frame, &mut controller.state))
            .context("redraw failed")?;
        if controller.should_quit {
            break;
        }
    }

    Ok(())
}

#[derive(Clone, Default)]
struct SharedStreamPacer {
    inner: Arc<std::sync::Mutex<StreamPacerState>>,
}

#[derive(Default)]
struct StreamPacerState {
    mode: StreamRenderMode,
    pending_chunks: usize,
    oldest_chunk_at: Option<std::time::Instant>,
}

impl SharedStreamPacer {
    fn note_enqueued(&self, count: usize) {
        let mut state = self.inner.lock().expect("stream pacer lock poisoned");
        if count == 0 {
            return;
        }
        if state.pending_chunks == 0 {
            state.oldest_chunk_at = Some(std::time::Instant::now());
        }
        state.pending_chunks += count;
    }

    fn note_consumed(&self, count: usize) {
        let mut state = self.inner.lock().expect("stream pacer lock poisoned");
        state.pending_chunks = state.pending_chunks.saturating_sub(count);
        if state.pending_chunks == 0 {
            state.oldest_chunk_at = None;
        }
    }

    fn update_mode(&self) -> (StreamRenderMode, usize, Duration) {
        let mut state = self.inner.lock().expect("stream pacer lock poisoned");
        let oldest = state
            .oldest_chunk_at
            .map(|instant| instant.elapsed())
            .unwrap_or(Duration::ZERO);
        state.mode = if state.pending_chunks >= 8 || oldest >= Duration::from_millis(200) {
            StreamRenderMode::CatchUp
        } else {
            StreamRenderMode::Smooth
        };
        (state.mode, state.pending_chunks, oldest)
    }

    fn mode(&self) -> StreamRenderMode {
        self.inner.lock().expect("stream pacer lock poisoned").mode
    }

    fn reset(&self) {
        let mut state = self.inner.lock().expect("stream pacer lock poisoned");
        *state = StreamPacerState::default();
    }
}

struct AppController<T = AstrcodeReqwestTransport> {
    client: AstrcodeClient<T>,
    state: CliState,
    debug_tap: Option<crate::launcher::DebugLogTap>,
    actions_tx: mpsc::UnboundedSender<Action>,
    actions_rx: mpsc::UnboundedReceiver<Action>,
    pending_session_id: Option<String>,
    pending_bootstrap_session_refresh: bool,
    stream_task: Option<JoinHandle<()>>,
    stream_pacer: SharedStreamPacer,
    should_quit: bool,
}

impl<T> AppController<T>
where
    T: AstrcodeClientTransport + 'static,
{
    fn new(
        client: AstrcodeClient<T>,
        state: CliState,
        debug_tap: Option<crate::launcher::DebugLogTap>,
        actions_tx: mpsc::UnboundedSender<Action>,
        actions_rx: mpsc::UnboundedReceiver<Action>,
    ) -> Self {
        Self {
            client,
            state,
            debug_tap,
            actions_tx,
            actions_rx,
            pending_session_id: None,
            pending_bootstrap_session_refresh: false,
            stream_task: None,
            stream_pacer: SharedStreamPacer::default(),
            should_quit: false,
        }
    }

    async fn bootstrap(&mut self) -> Result<()> {
        self.pending_bootstrap_session_refresh = true;
        self.execute_command(crate::command::Command::New).await;
        Ok(())
    }

    fn stop_background_tasks(&mut self) {
        if let Some(stream_task) = self.stream_task.take() {
            stream_task.abort();
        }
    }

    async fn handle_action(&mut self, action: Action) -> Result<()> {
        match action {
            Action::Tick => {
                if let Some(debug_tap) = &self.debug_tap {
                    for line in debug_tap.drain() {
                        self.state.push_debug_line(line);
                    }
                }
                let (mode, pending, oldest) = self.stream_pacer.update_mode();
                self.state.set_stream_mode(mode, pending, oldest);
            },
            Action::Quit => self.should_quit = true,
            Action::Resize { width, height } => self.state.set_viewport_size(width, height),
            Action::Key(key) => self.handle_key(key).await?,
            Action::Paste(text) => self.handle_paste(text).await?,
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
                    let mut sessions = self.state.conversation.sessions.clone();
                    sessions.retain(|existing| existing.session_id != session_id);
                    sessions.push(session);
                    sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
                    self.state.update_sessions(sessions);
                    self.begin_session_hydration(session_id).await;
                },
                Err(error) => {
                    self.apply_status_error(error);
                    if self.pending_bootstrap_session_refresh {
                        self.pending_bootstrap_session_refresh = false;
                        self.refresh_sessions().await;
                    }
                },
            },
            Action::SnapshotLoaded { session_id, result } => {
                if !self.pending_session_matches(session_id.as_str()) {
                    return Ok(());
                }
                match result {
                    Ok(snapshot) => {
                        self.pending_session_id = None;
                        self.state.activate_snapshot(snapshot);
                        self.state
                            .set_status(format!("attached to session {}", session_id));
                        self.open_stream_for_active_session().await;
                        if self.pending_bootstrap_session_refresh {
                            self.pending_bootstrap_session_refresh = false;
                            self.refresh_sessions().await;
                        }
                    },
                    Err(error) => {
                        self.pending_session_id = None;
                        self.apply_hydration_error(error);
                        if self.pending_bootstrap_session_refresh {
                            self.pending_bootstrap_session_refresh = false;
                            self.refresh_sessions().await;
                        }
                    },
                }
            },
            Action::StreamBatch { session_id, items } => {
                let batch_len = items.len();
                if !self.active_session_matches(session_id.as_str()) {
                    self.stream_pacer.note_consumed(batch_len);
                    return Ok(());
                }
                for item in items {
                    self.apply_stream_event(session_id.as_str(), item).await;
                }
                self.stream_pacer.note_consumed(batch_len);
                let (mode, pending, oldest) = self.stream_pacer.update_mode();
                self.state.set_stream_mode(mode, pending, oldest);
            },
            Action::SlashCandidatesLoaded { query, result } => {
                let OverlayState::SlashPalette(palette) = &self.state.interaction.overlay else {
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
                if !self.active_session_matches(session_id.as_str()) {
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
                if !self.active_session_matches(session_id.as_str()) {
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
            KeyCode::F(2) => self.state.toggle_debug_overlay(),
            KeyCode::Esc => self.state.close_overlay(),
            KeyCode::Left => {
                if !matches!(self.state.interaction.overlay, OverlayState::None) {
                    return Ok(());
                }
                self.state.cycle_focus_backward();
            },
            KeyCode::Right => {
                if !matches!(self.state.interaction.overlay, OverlayState::None) {
                    return Ok(());
                }
                self.state.cycle_focus_forward();
            },
            KeyCode::Up => {
                if !matches!(self.state.interaction.overlay, OverlayState::None) {
                    self.state.overlay_prev();
                } else if matches!(self.state.interaction.pane_focus, PaneFocus::ChildPane) {
                    self.state.child_prev();
                } else {
                    self.state.scroll_up();
                }
            },
            KeyCode::Down => {
                if !matches!(self.state.interaction.overlay, OverlayState::None) {
                    self.state.overlay_next();
                } else if matches!(self.state.interaction.pane_focus, PaneFocus::ChildPane) {
                    self.state.child_next();
                } else {
                    self.state.scroll_down();
                }
            },
            KeyCode::Enter => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    if matches!(self.state.interaction.overlay, OverlayState::None)
                        && matches!(self.state.interaction.pane_focus, PaneFocus::Composer)
                    {
                        self.state.insert_newline();
                    }
                } else if let Some(selection) = self.state.selected_overlay() {
                    self.execute_overlay_action(overlay_action(selection))
                        .await?;
                } else if matches!(self.state.interaction.pane_focus, PaneFocus::ChildPane) {
                    self.state.toggle_child_focus();
                } else {
                    self.submit_current_input().await;
                }
            },
            KeyCode::Backspace => {
                if matches!(self.state.interaction.overlay, OverlayState::None) {
                    self.state.pop_input();
                } else {
                    self.state.overlay_query_pop();
                    self.refresh_overlay_query().await;
                }
            },
            KeyCode::Tab => {
                let query = self.slash_query_for_current_input();
                self.open_slash_palette(query).await;
            },
            KeyCode::Char(ch) => {
                if matches!(self.state.interaction.overlay, OverlayState::None) {
                    self.state.interaction.pane_focus = PaneFocus::Composer;
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

    async fn handle_paste(&mut self, text: String) -> Result<()> {
        if matches!(self.state.interaction.overlay, OverlayState::None) {
            self.state.interaction.pane_focus = PaneFocus::Composer;
            self.state.append_input(text.as_str());
        } else {
            self.state.overlay_query_append(text.as_str());
            self.refresh_overlay_query().await;
        }
        Ok(())
    }

    fn active_session_matches(&self, session_id: &str) -> bool {
        self.state.conversation.active_session_id.as_deref() == Some(session_id)
    }

    fn pending_session_matches(&self, session_id: &str) -> bool {
        self.pending_session_id.as_deref() == Some(session_id)
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
                        Ok(CrosstermEvent::Paste(text)) => {
                            if actions_tx.send(Action::Paste(text)).is_err() {
                                break;
                            }
                        },
                        Ok(CrosstermEvent::Resize(width, height)) => {
                            if actions_tx.send(Action::Resize { width, height }).is_err() {
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
        .shell
        .working_dir
        .as_deref()
        .context("working directory is required for /new")
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
    use std::{
        collections::VecDeque,
        sync::{Arc, Mutex},
    };

    use astrcode_client::{
        AstrcodeClientTransport, AstrcodePhaseDto, AstrcodeSseEvent, AstrcodeTransportError,
        AstrcodeTransportMethod, AstrcodeTransportRequest, AstrcodeTransportResponse,
    };
    use async_trait::async_trait;
    use serde_json::json;
    use tokio::{sync::mpsc, time::timeout};

    use super::*;
    use crate::{
        capability::{ColorLevel, GlyphMode, TerminalCapabilities},
        command::Command,
    };

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

    fn ascii_capabilities() -> TerminalCapabilities {
        TerminalCapabilities {
            color: ColorLevel::None,
            glyphs: GlyphMode::Ascii,
            alt_screen: false,
            mouse: false,
            bracketed_paste: false,
        }
    }

    #[derive(Debug)]
    enum MockCall {
        Request {
            expected: AstrcodeTransportRequest,
            result: Result<AstrcodeTransportResponse, AstrcodeTransportError>,
        },
        Stream {
            expected: AstrcodeTransportRequest,
            events: Vec<Result<AstrcodeSseEvent, AstrcodeTransportError>>,
        },
    }

    #[derive(Debug, Default, Clone)]
    struct MockTransport {
        calls: Arc<Mutex<VecDeque<MockCall>>>,
    }

    impl MockTransport {
        fn push(&self, call: MockCall) {
            self.calls
                .lock()
                .expect("mock lock poisoned")
                .push_back(call);
        }

        fn assert_consumed(&self) {
            assert!(
                self.calls.lock().expect("mock lock poisoned").is_empty(),
                "all mocked transport calls should be consumed"
            );
        }
    }

    #[async_trait]
    impl AstrcodeClientTransport for MockTransport {
        async fn execute(
            &self,
            request: AstrcodeTransportRequest,
        ) -> Result<AstrcodeTransportResponse, AstrcodeTransportError> {
            let Some(MockCall::Request { expected, result }) =
                self.calls.lock().expect("mock lock poisoned").pop_front()
            else {
                panic!("expected request call");
            };
            assert_eq!(request, expected);
            result
        }

        async fn open_sse(
            &self,
            request: AstrcodeTransportRequest,
            buffer: usize,
        ) -> Result<
            tokio::sync::mpsc::Receiver<Result<AstrcodeSseEvent, AstrcodeTransportError>>,
            AstrcodeTransportError,
        > {
            let Some(MockCall::Stream { expected, events }) =
                self.calls.lock().expect("mock lock poisoned").pop_front()
            else {
                panic!("expected stream call");
            };
            assert_eq!(request, expected);
            let (sender, receiver) = mpsc::channel(buffer.max(1));
            tokio::spawn(async move {
                for event in events {
                    let _ = sender.send(event).await;
                }
            });
            Ok(receiver)
        }
    }

    fn client_with_transport(transport: MockTransport) -> AstrcodeClient<MockTransport> {
        AstrcodeClient::with_transport(
            ClientConfig {
                origin: "http://localhost:5529".to_string(),
                api_token: Some("session-token".to_string()),
                api_token_expires_at_ms: None,
                stream_buffer: 8,
            },
            transport,
        )
    }

    fn snapshot_response(session_id: &str, title: &str) -> AstrcodeTransportResponse {
        AstrcodeTransportResponse {
            status: 200,
            body: json!({
                "sessionId": session_id,
                "sessionTitle": title,
                "cursor": format!("cursor:{session_id}"),
                "phase": "idle",
                "control": {
                    "phase": "idle",
                    "canSubmitPrompt": true,
                    "canRequestCompact": true,
                    "compactPending": false
                },
                "blocks": [{
                    "kind": "assistant",
                    "id": format!("assistant:{session_id}"),
                    "status": "complete",
                    "markdown": format!("hydrated {session_id}")
                }],
                "childSummaries": [],
                "slashCandidates": [],
                "banner": null
            })
            .to_string(),
        }
    }

    async fn handle_next_action<T>(controller: &mut AppController<T>)
    where
        T: AstrcodeClientTransport + 'static,
    {
        let action = timeout(Duration::from_millis(200), controller.actions_rx.recv())
            .await
            .expect("pending action should arrive")
            .expect("action channel should stay open");
        controller
            .handle_action(action)
            .await
            .expect("handling queued action should succeed");
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

    #[tokio::test]
    async fn bootstrap_creates_fresh_session_instead_of_restoring_existing_one() {
        let transport = MockTransport::default();
        let existing = session("session-old", "D:/repo-a", "old", "2026-04-15T10:00:00Z");
        let created = session("session-new", "D:/repo-a", "new", "2026-04-15T12:30:00Z");

        transport.push(MockCall::Request {
            expected: AstrcodeTransportRequest {
                method: AstrcodeTransportMethod::Post,
                url: "http://localhost:5529/api/sessions".to_string(),
                auth_token: Some("session-token".to_string()),
                query: Vec::new(),
                json_body: Some(json!({
                    "workingDir": "D:/repo-a"
                })),
            },
            result: Ok(AstrcodeTransportResponse {
                status: 201,
                body: serde_json::to_string(&created).expect("session should serialize"),
            }),
        });
        transport.push(MockCall::Request {
            expected: AstrcodeTransportRequest {
                method: AstrcodeTransportMethod::Get,
                url: "http://localhost:5529/api/v1/conversation/sessions/session-new/snapshot"
                    .to_string(),
                auth_token: Some("session-token".to_string()),
                query: Vec::new(),
                json_body: None,
            },
            result: Ok(snapshot_response("session-new", "new")),
        });
        transport.push(MockCall::Stream {
            expected: AstrcodeTransportRequest {
                method: AstrcodeTransportMethod::Get,
                url: "http://localhost:5529/api/v1/conversation/sessions/session-new/stream"
                    .to_string(),
                auth_token: Some("session-token".to_string()),
                query: vec![("cursor".to_string(), "cursor:session-new".to_string())],
                json_body: None,
            },
            events: Vec::new(),
        });
        transport.push(MockCall::Request {
            expected: AstrcodeTransportRequest {
                method: AstrcodeTransportMethod::Get,
                url: "http://localhost:5529/api/sessions".to_string(),
                auth_token: Some("session-token".to_string()),
                query: Vec::new(),
                json_body: None,
            },
            result: Ok(AstrcodeTransportResponse {
                status: 200,
                body: serde_json::to_string(&vec![created.clone(), existing])
                    .expect("sessions should serialize"),
            }),
        });

        let (actions_tx, actions_rx) = mpsc::unbounded_channel();
        let mut controller = AppController::new(
            client_with_transport(transport.clone()),
            CliState::new(
                "http://localhost:5529".to_string(),
                Some(PathBuf::from("D:/repo-a")),
                ascii_capabilities(),
            ),
            None,
            actions_tx,
            actions_rx,
        );
        controller.state.update_sessions(vec![session(
            "session-old",
            "D:/repo-a",
            "old",
            "2026-04-15T10:00:00Z",
        )]);

        controller
            .bootstrap()
            .await
            .expect("bootstrap should succeed");
        handle_next_action(&mut controller).await;
        handle_next_action(&mut controller).await;
        handle_next_action(&mut controller).await;

        assert_eq!(
            controller.state.conversation.active_session_id.as_deref(),
            Some("session-new")
        );
        assert!(
            controller
                .state
                .conversation
                .sessions
                .iter()
                .any(|session| session.session_id == "session-new"),
            "bootstrap should attach the freshly created session"
        );
        transport.assert_consumed();
    }

    #[tokio::test]
    async fn bootstrap_create_failure_surfaces_error_without_attaching_old_session() {
        let transport = MockTransport::default();
        let existing = session("session-old", "D:/repo-a", "old", "2026-04-15T10:00:00Z");

        transport.push(MockCall::Request {
            expected: AstrcodeTransportRequest {
                method: AstrcodeTransportMethod::Post,
                url: "http://localhost:5529/api/sessions".to_string(),
                auth_token: Some("session-token".to_string()),
                query: Vec::new(),
                json_body: Some(json!({
                    "workingDir": "D:/repo-a"
                })),
            },
            result: Err(AstrcodeTransportError::Http {
                status: 500,
                body: json!({
                    "code": "transport_unavailable",
                    "message": "create failed"
                })
                .to_string(),
            }),
        });
        transport.push(MockCall::Request {
            expected: AstrcodeTransportRequest {
                method: AstrcodeTransportMethod::Get,
                url: "http://localhost:5529/api/sessions".to_string(),
                auth_token: Some("session-token".to_string()),
                query: Vec::new(),
                json_body: None,
            },
            result: Ok(AstrcodeTransportResponse {
                status: 200,
                body: serde_json::to_string(&vec![existing.clone()])
                    .expect("sessions should serialize"),
            }),
        });

        let (actions_tx, actions_rx) = mpsc::unbounded_channel();
        let mut controller = AppController::new(
            client_with_transport(transport.clone()),
            CliState::new(
                "http://localhost:5529".to_string(),
                Some(PathBuf::from("D:/repo-a")),
                ascii_capabilities(),
            ),
            None,
            actions_tx,
            actions_rx,
        );
        controller.state.update_sessions(vec![existing]);

        controller
            .bootstrap()
            .await
            .expect("bootstrap should succeed");
        handle_next_action(&mut controller).await;
        handle_next_action(&mut controller).await;

        assert_eq!(controller.state.conversation.active_session_id, None);
        assert!(controller.state.interaction.status.is_error);
        assert_eq!(controller.state.interaction.status.message, "create failed");
        transport.assert_consumed();
    }

    #[tokio::test]
    async fn end_to_end_acceptance_covers_resume_compact_skill_and_single_active_stream_switch() {
        let transport = MockTransport::default();
        let session_one = session("session-1", "D:/repo-a", "repo-a", "2026-04-15T10:00:00Z");
        let session_two = session("session-2", "D:/repo-b", "repo-b", "2026-04-15T12:00:00Z");

        transport.push(MockCall::Request {
            expected: AstrcodeTransportRequest {
                method: AstrcodeTransportMethod::Get,
                url: "http://localhost:5529/api/v1/conversation/sessions/session-1/snapshot"
                    .to_string(),
                auth_token: Some("session-token".to_string()),
                query: Vec::new(),
                json_body: None,
            },
            result: Ok(snapshot_response("session-1", "repo-a")),
        });
        transport.push(MockCall::Stream {
            expected: AstrcodeTransportRequest {
                method: AstrcodeTransportMethod::Get,
                url: "http://localhost:5529/api/v1/conversation/sessions/session-1/stream"
                    .to_string(),
                auth_token: Some("session-token".to_string()),
                query: vec![("cursor".to_string(), "cursor:session-1".to_string())],
                json_body: None,
            },
            events: Vec::new(),
        });
        transport.push(MockCall::Request {
            expected: AstrcodeTransportRequest {
                method: AstrcodeTransportMethod::Get,
                url:
                    "http://localhost:5529/api/v1/conversation/sessions/session-1/slash-candidates"
                        .to_string(),
                auth_token: Some("session-token".to_string()),
                query: vec![("q".to_string(), "review".to_string())],
                json_body: None,
            },
            result: Ok(AstrcodeTransportResponse {
                status: 200,
                body: json!({
                    "items": [{
                        "id": "skill-review",
                        "title": "Review skill",
                        "description": "插入 review skill",
                        "keywords": ["review"],
                        "actionKind": "insert_text",
                        "actionValue": "/skill review"
                    }]
                })
                .to_string(),
            }),
        });
        transport.push(MockCall::Request {
            expected: AstrcodeTransportRequest {
                method: AstrcodeTransportMethod::Post,
                url: "http://localhost:5529/api/sessions/session-1/compact".to_string(),
                auth_token: Some("session-token".to_string()),
                query: Vec::new(),
                json_body: Some(json!({
                    "control": {
                        "manualCompact": true
                    }
                })),
            },
            result: Ok(AstrcodeTransportResponse {
                status: 202,
                body: json!({
                    "accepted": true,
                    "deferred": false,
                    "message": "手动 compact 已执行。"
                })
                .to_string(),
            }),
        });
        transport.push(MockCall::Request {
            expected: AstrcodeTransportRequest {
                method: AstrcodeTransportMethod::Get,
                url: "http://localhost:5529/api/sessions".to_string(),
                auth_token: Some("session-token".to_string()),
                query: Vec::new(),
                json_body: None,
            },
            result: Ok(AstrcodeTransportResponse {
                status: 200,
                body: serde_json::to_string(&vec![session_one.clone(), session_two.clone()])
                    .expect("sessions should serialize"),
            }),
        });
        transport.push(MockCall::Request {
            expected: AstrcodeTransportRequest {
                method: AstrcodeTransportMethod::Get,
                url: "http://localhost:5529/api/v1/conversation/sessions/session-2/snapshot"
                    .to_string(),
                auth_token: Some("session-token".to_string()),
                query: Vec::new(),
                json_body: None,
            },
            result: Ok(snapshot_response("session-2", "repo-b")),
        });
        transport.push(MockCall::Stream {
            expected: AstrcodeTransportRequest {
                method: AstrcodeTransportMethod::Get,
                url: "http://localhost:5529/api/v1/conversation/sessions/session-2/stream"
                    .to_string(),
                auth_token: Some("session-token".to_string()),
                query: vec![("cursor".to_string(), "cursor:session-2".to_string())],
                json_body: None,
            },
            events: Vec::new(),
        });

        let (actions_tx, actions_rx) = mpsc::unbounded_channel();
        let mut controller = AppController::new(
            client_with_transport(transport.clone()),
            CliState::new(
                "http://localhost:5529".to_string(),
                Some(PathBuf::from("D:/repo-a")),
                ascii_capabilities(),
            ),
            None,
            actions_tx,
            actions_rx,
        );
        controller
            .state
            .update_sessions(vec![session_one.clone(), session_two.clone()]);

        controller
            .begin_session_hydration("session-1".to_string())
            .await;
        handle_next_action(&mut controller).await;
        assert_eq!(
            controller.state.conversation.active_session_id.as_deref(),
            Some("session-1")
        );
        assert_eq!(
            controller.state.conversation.transcript.len(),
            1,
            "session one should hydrate one transcript block"
        );

        controller
            .execute_command(Command::Skill {
                query: Some("review".to_string()),
            })
            .await;
        handle_next_action(&mut controller).await;
        let OverlayState::SlashPalette(palette) = &controller.state.interaction.overlay else {
            panic!("skill command should open slash palette");
        };
        assert_eq!(palette.query, "review");
        assert_eq!(palette.items.len(), 1);

        controller.execute_command(Command::Compact).await;
        handle_next_action(&mut controller).await;
        assert_eq!(
            controller.state.interaction.status.message,
            "手动 compact 已执行。"
        );

        controller
            .execute_command(Command::Resume {
                query: Some("repo-b".to_string()),
            })
            .await;
        let OverlayState::Resume(resume) = &controller.state.interaction.overlay else {
            panic!("resume command should open resume overlay");
        };
        assert_eq!(resume.query, "repo-b");
        handle_next_action(&mut controller).await;
        let selection = controller
            .state
            .selected_overlay()
            .expect("resume overlay should keep a selection");
        controller
            .execute_overlay_action(overlay_action(selection))
            .await
            .expect("resume selection should switch session");
        handle_next_action(&mut controller).await;
        assert_eq!(
            controller.state.conversation.active_session_id.as_deref(),
            Some("session-2")
        );
        assert!(
            controller
                .state
                .conversation
                .transcript
                .iter()
                .any(|block| matches!(
                    block,
                    astrcode_client::AstrcodeConversationBlockDto::Assistant(block)
                        if block.id == "assistant:session-2"
                )),
            "session two snapshot should replace transcript"
        );

        let transcript_before = controller.state.conversation.transcript.clone();
        controller
            .handle_action(Action::StreamBatch {
                session_id: "session-1".to_string(),
                items: vec![ConversationStreamItem::Delta(Box::new(
                    astrcode_client::AstrcodeConversationStreamEnvelopeDto {
                        session_id: "session-1".to_string(),
                        cursor: astrcode_client::AstrcodeConversationCursorDto(
                            "cursor:old".to_string(),
                        ),
                        delta: astrcode_client::AstrcodeConversationDeltaDto::AppendBlock {
                            block: astrcode_client::AstrcodeConversationBlockDto::Assistant(
                                astrcode_client::AstrcodeConversationAssistantBlockDto {
                                    id: "assistant:stale".to_string(),
                                    turn_id: None,
                                    status:
                                        astrcode_client::AstrcodeConversationBlockStatusDto::Complete,
                                    markdown: "stale".to_string(),
                                },
                            ),
                        },
                    },
                ))],
            })
            .await
            .expect("stale batch should be ignored");
        assert_eq!(
            controller.state.conversation.transcript, transcript_before,
            "single active stream mode should ignore deltas from inactive sessions"
        );

        transport.assert_consumed();
    }
}
