mod bottom_pane;
mod cells;
mod overlay;
mod theme;

pub use bottom_pane::{BottomPaneView, ComposerPane};
pub use cells::{RenderableCell, wrap_text};
pub use overlay::{OverlayView, overlay_title};
use ratatui::text::{Line, Span};
pub use theme::{CodexTheme, ThemePalette};

use crate::{
    capability::TerminalCapabilities,
    state::{CliState, OverlayState, WrappedLine, WrappedLineStyle},
};

pub fn transcript_lines(state: &CliState, width: u16) -> Vec<WrappedLine> {
    if state.conversation.transcript_cells.is_empty() {
        return empty_state_lines(state, width);
    }

    let theme = CodexTheme::new(state.shell.capabilities);
    let mut lines = Vec::new();
    for cell in &state.conversation.transcript_cells {
        lines.extend(cell.render_lines(
            usize::from(width.max(18)),
            state.shell.capabilities,
            &theme,
        ));
    }
    lines
}

pub fn child_pane_lines(state: &CliState, width: u16) -> Vec<WrappedLine> {
    let theme = CodexTheme::new(state.shell.capabilities);
    let width = usize::from(width.max(18));
    let mut lines = vec![
        WrappedLine {
            style: WrappedLineStyle::Header,
            content: "child sessions".to_string(),
        },
        WrappedLine {
            style: WrappedLineStyle::Dim,
            content: format!(
                "{} active  ·  {} total",
                state
                    .conversation
                    .child_summaries
                    .iter()
                    .filter(|child| {
                        matches!(
                            child.lifecycle,
                            astrcode_client::AstrcodeConversationAgentLifecycleDto::Running
                                | astrcode_client::AstrcodeConversationAgentLifecycleDto::Pending
                        )
                    })
                    .count(),
                state.conversation.child_summaries.len()
            ),
        },
        WrappedLine {
            style: WrappedLineStyle::Border,
            content: theme.divider().repeat(width),
        },
    ];

    for (index, child) in state.conversation.child_summaries.iter().enumerate() {
        let focused = state
            .interaction
            .child_pane
            .focused_child_session_id
            .as_deref()
            == Some(child.child_session_id.as_str());
        let selected = index == state.interaction.child_pane.selected;
        let marker = if focused {
            theme.glyph("◆", "*")
        } else if selected {
            theme.glyph("›", ">")
        } else {
            " "
        };
        lines.push(WrappedLine {
            style: if focused || selected {
                WrappedLineStyle::Selection
            } else {
                WrappedLineStyle::Plain
            },
            content: format!(
                "{marker} {}  [{}]",
                child.title,
                lifecycle_label(child.lifecycle)
            ),
        });
        if let Some(summary) = child.latest_output_summary.as_deref() {
            for line in wrap_text(
                summary,
                width.saturating_sub(2).max(8),
                state.shell.capabilities,
            ) {
                lines.push(WrappedLine {
                    style: WrappedLineStyle::Dim,
                    content: format!("  {line}"),
                });
            }
        }
    }

    lines
}

pub fn header_lines(state: &CliState, width: u16) -> Vec<WrappedLine> {
    let theme = CodexTheme::new(state.shell.capabilities);
    let title = state
        .conversation
        .active_session_title
        .as_deref()
        .unwrap_or("Astrcode workspace");
    let model = "gpt-5.4 medium";
    let phase = phase_label(
        state
            .active_phase()
            .unwrap_or(astrcode_client::AstrcodePhaseDto::Idle),
    );
    let working_dir = state
        .shell
        .working_dir
        .as_deref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "~".to_string());

    let meta = format!("model {model}  ·  phase {phase}  ·  {working_dir}");
    let meta_width = usize::from(width.max(20)).saturating_sub(2);

    vec![
        WrappedLine {
            style: WrappedLineStyle::Header,
            content: title.to_string(),
        },
        WrappedLine {
            style: WrappedLineStyle::Dim,
            content: truncate_text(meta.as_str(), meta_width, state.shell.capabilities),
        },
        WrappedLine {
            style: WrappedLineStyle::Border,
            content: theme.divider().repeat(usize::from(width.max(1))),
        },
    ]
}

pub fn centered_overlay_lines(state: &CliState, width: u16) -> Vec<WrappedLine> {
    let theme = CodexTheme::new(state.shell.capabilities);
    match &state.interaction.overlay {
        OverlayState::Resume(resume) => {
            resume.lines(usize::from(width.max(20)), state.shell.capabilities, &theme)
        },
        OverlayState::DebugLogs(debug) => debug.lines(
            usize::from(width.max(20)),
            state.shell.capabilities,
            &theme,
            &state.debug,
        ),
        OverlayState::SlashPalette(_) | OverlayState::None => Vec::new(),
    }
}

pub fn line_to_ratatui(line: &WrappedLine, capabilities: TerminalCapabilities) -> Line<'static> {
    let theme = CodexTheme::new(capabilities);
    Line::from(Span::styled(
        line.content.clone(),
        theme.line_style(line.style),
    ))
}

pub fn phase_label(phase: astrcode_client::AstrcodePhaseDto) -> &'static str {
    match phase {
        astrcode_client::AstrcodePhaseDto::Idle => "idle",
        astrcode_client::AstrcodePhaseDto::Thinking => "thinking",
        astrcode_client::AstrcodePhaseDto::CallingTool => "calling_tool",
        astrcode_client::AstrcodePhaseDto::Streaming => "streaming",
        astrcode_client::AstrcodePhaseDto::Interrupted => "interrupted",
        astrcode_client::AstrcodePhaseDto::Done => "done",
    }
}

fn empty_state_lines(state: &CliState, width: u16) -> Vec<WrappedLine> {
    let theme = CodexTheme::new(state.shell.capabilities);
    let divider = theme.divider().repeat(usize::from(width.min(56)));
    vec![
        WrappedLine {
            style: WrappedLineStyle::Header,
            content: "OpenAI Codex style workspace".to_string(),
        },
        WrappedLine {
            style: WrappedLineStyle::Dim,
            content: "fresh session 已准备好。主区只显示会话语义内容，启动噪音已移出。".to_string(),
        },
        WrappedLine {
            style: WrappedLineStyle::Accent,
            content: "› 输入 prompt 开始；Tab 打开 commands；F2 查看 debug logs。".to_string(),
        },
        WrappedLine {
            style: WrappedLineStyle::Border,
            content: divider,
        },
    ]
}

fn lifecycle_label(
    lifecycle: astrcode_client::AstrcodeConversationAgentLifecycleDto,
) -> &'static str {
    match lifecycle {
        astrcode_client::AstrcodeConversationAgentLifecycleDto::Pending => "pending",
        astrcode_client::AstrcodeConversationAgentLifecycleDto::Running => "running",
        astrcode_client::AstrcodeConversationAgentLifecycleDto::Idle => "idle",
        astrcode_client::AstrcodeConversationAgentLifecycleDto::Terminated => "terminated",
    }
}

fn truncate_text(text: &str, width: usize, capabilities: TerminalCapabilities) -> String {
    if width == 0 {
        return String::new();
    }

    let mut out = String::new();
    let mut current_width = 0;
    for ch in text.chars() {
        let ch = ch.to_string();
        let ch_width = if capabilities.ascii_only() {
            1
        } else {
            unicode_width::UnicodeWidthStr::width(ch.as_str()).max(1)
        };
        if current_width + ch_width > width {
            break;
        }
        current_width += ch_width;
        out.push_str(ch.as_str());
    }
    out
}
