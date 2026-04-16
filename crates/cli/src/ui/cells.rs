use unicode_width::UnicodeWidthStr;

use super::theme::ThemePalette;
use crate::{
    capability::TerminalCapabilities,
    state::{
        TranscriptCell, TranscriptCellKind, TranscriptCellStatus, WrappedLine, WrappedLineStyle,
    },
};

const LIVE_PREFIX: usize = 2;

pub trait RenderableCell {
    fn render_lines(
        &self,
        width: usize,
        capabilities: TerminalCapabilities,
        theme: &dyn ThemePalette,
    ) -> Vec<WrappedLine>;
}

impl RenderableCell for TranscriptCell {
    fn render_lines(
        &self,
        width: usize,
        capabilities: TerminalCapabilities,
        theme: &dyn ThemePalette,
    ) -> Vec<WrappedLine> {
        let width = width.max(20);
        match &self.kind {
            TranscriptCellKind::User { body } => render_user_cell(body, width, capabilities, theme),
            TranscriptCellKind::Assistant { body, status } => render_labeled_cell(
                width,
                capabilities,
                theme,
                &format!(
                    "{} Astrcode{}",
                    theme.glyph("●", "*"),
                    status_suffix(*status)
                ),
                body,
                WrappedLineStyle::Header,
                WrappedLineStyle::Plain,
            ),
            TranscriptCellKind::Thinking { body, status } => render_labeled_cell(
                width,
                capabilities,
                theme,
                &format!(
                    "{} thinking{}",
                    theme.glyph("◌", "o"),
                    status_suffix(*status)
                ),
                body,
                WrappedLineStyle::Dim,
                WrappedLineStyle::Dim,
            ),
            TranscriptCellKind::ToolCall {
                tool_name,
                summary,
                status,
                stdout,
                stderr,
                error,
                duration_ms,
                truncated,
                child_session_id,
            } => render_labeled_cell(
                width,
                capabilities,
                theme,
                &format!(
                    "{} tool {}{}",
                    theme.glyph("↳", ">"),
                    tool_name,
                    status_suffix(*status)
                ),
                &tool_call_body(
                    summary,
                    stdout,
                    stderr,
                    error.as_deref(),
                    *duration_ms,
                    *truncated,
                    child_session_id.as_deref(),
                ),
                WrappedLineStyle::Warning,
                WrappedLineStyle::Dim,
            ),
            TranscriptCellKind::Error { code, message } => render_labeled_cell(
                width,
                capabilities,
                theme,
                &format!("{} error {code}", theme.glyph("✕", "x")),
                message,
                WrappedLineStyle::Error,
                WrappedLineStyle::Error,
            ),
            TranscriptCellKind::SystemNote {
                note_kind,
                markdown,
            } => render_labeled_cell(
                width,
                capabilities,
                theme,
                &format!("{} {note_kind}", theme.glyph("·", "-")),
                markdown,
                WrappedLineStyle::Dim,
                WrappedLineStyle::Dim,
            ),
            TranscriptCellKind::ChildHandoff {
                handoff_kind,
                title,
                lifecycle,
                message,
                child_session_id,
                child_agent_id,
            } => {
                let mut lines = render_labeled_cell(
                    width,
                    capabilities,
                    theme,
                    &format!(
                        "{} child {} [{} / {}]",
                        theme.glyph("◇", "*"),
                        title,
                        handoff_kind,
                        lifecycle_label(*lifecycle)
                    ),
                    message,
                    WrappedLineStyle::Accent,
                    WrappedLineStyle::Plain,
                );
                lines.extend([
                    prefixed_line(
                        WrappedLineStyle::Dim,
                        &format!("session {child_session_id}"),
                        capabilities,
                        width,
                    ),
                    prefixed_line(
                        WrappedLineStyle::Dim,
                        &format!("agent   {child_agent_id}"),
                        capabilities,
                        width,
                    ),
                    WrappedLine {
                        style: WrappedLineStyle::Plain,
                        content: String::new(),
                    },
                ]);
                lines
            },
        }
    }
}

fn tool_call_body(
    summary: &str,
    stdout: &str,
    stderr: &str,
    error: Option<&str>,
    duration_ms: Option<u64>,
    truncated: bool,
    child_session_id: Option<&str>,
) -> String {
    let mut sections = Vec::new();
    if !summary.trim().is_empty() {
        sections.push(summary.trim().to_string());
    }
    if !stdout.trim().is_empty() {
        sections.push(format!("stdout:\n{}", stdout.trim_end()));
    }
    if !stderr.trim().is_empty() {
        sections.push(format!("stderr:\n{}", stderr.trim_end()));
    }
    if let Some(error) = error.filter(|value| !value.trim().is_empty()) {
        sections.push(format!("error: {}", error.trim()));
    }
    if let Some(duration_ms) = duration_ms {
        sections.push(format!("duration: {duration_ms} ms"));
    }
    if truncated {
        sections.push("truncated: true".to_string());
    }
    if let Some(child_session_id) = child_session_id.filter(|value| !value.trim().is_empty()) {
        sections.push(format!("child session: {child_session_id}"));
    }
    if sections.is_empty() {
        return "正在执行工具调用".to_string();
    }
    sections.join("\n\n")
}

fn render_user_cell(
    body: &str,
    width: usize,
    capabilities: TerminalCapabilities,
    theme: &dyn ThemePalette,
) -> Vec<WrappedLine> {
    let prefix = format!("{} ", theme.glyph("▌", "|"));
    let body_width = width.saturating_sub(display_width(prefix.as_str(), capabilities));
    let wrapped = wrap_text(body, body_width.max(8), capabilities);
    let mut lines = Vec::with_capacity(wrapped.len() + 1);
    for line in wrapped {
        lines.push(WrappedLine {
            style: WrappedLineStyle::User,
            content: format!("{prefix}{line}"),
        });
    }
    lines.push(WrappedLine {
        style: WrappedLineStyle::Plain,
        content: String::new(),
    });
    lines
}

fn render_labeled_cell(
    width: usize,
    capabilities: TerminalCapabilities,
    _theme: &dyn ThemePalette,
    title: &str,
    body: &str,
    title_style: WrappedLineStyle,
    body_style: WrappedLineStyle,
) -> Vec<WrappedLine> {
    let mut lines = vec![prefixed_line(title_style, title, capabilities, width)];
    for line in wrap_text(body, width.saturating_sub(LIVE_PREFIX).max(8), capabilities) {
        lines.push(prefixed_line(
            body_style,
            line.as_str(),
            capabilities,
            width,
        ));
    }
    lines.push(WrappedLine {
        style: WrappedLineStyle::Plain,
        content: String::new(),
    });
    lines
}

fn prefixed_line(
    style: WrappedLineStyle,
    content: &str,
    capabilities: TerminalCapabilities,
    width: usize,
) -> WrappedLine {
    let prefix = "  ";
    let available = width.saturating_sub(display_width(prefix, capabilities));
    let text = truncate_to_width(content, available.max(1), capabilities);
    WrappedLine {
        style,
        content: format!("{prefix}{text}"),
    }
}

pub fn wrap_text(text: &str, width: usize, capabilities: TerminalCapabilities) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    let normalized = if text.trim().is_empty() {
        vec![String::new()]
    } else {
        text.lines().map(ToString::to_string).collect::<Vec<_>>()
    };

    for raw_line in normalized {
        if raw_line.is_empty() {
            out.push(String::new());
            continue;
        }

        let words = raw_line.split_whitespace().collect::<Vec<_>>();
        if words.len() <= 1 {
            wrap_by_width(raw_line.as_str(), width, capabilities, &mut out);
            continue;
        }

        let mut current = String::new();
        for word in words {
            let candidate = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            if display_width(candidate.as_str(), capabilities) > width && !current.is_empty() {
                out.push(current);
                current = String::new();
            }

            if current.is_empty() && display_width(word, capabilities) > width {
                wrap_by_width(word, width, capabilities, &mut out);
            } else if current.is_empty() {
                current = word.to_string();
            } else {
                current = format!("{current} {word}");
            }
        }

        if !current.is_empty() {
            out.push(current);
        }
    }

    out
}

fn wrap_by_width(
    text: &str,
    width: usize,
    capabilities: TerminalCapabilities,
    out: &mut Vec<String>,
) {
    let mut current = String::new();
    let mut current_width = 0;
    for ch in text.chars() {
        let ch_width = display_width(ch.to_string().as_str(), capabilities).max(1);
        if current_width + ch_width > width && !current.is_empty() {
            out.push(std::mem::take(&mut current));
            current_width = 0;
        }
        current.push(ch);
        current_width += ch_width;
    }
    if !current.is_empty() {
        out.push(current);
    }
}

fn truncate_to_width(text: &str, width: usize, capabilities: TerminalCapabilities) -> String {
    let mut out = String::new();
    let mut current_width = 0;
    for ch in text.chars() {
        let ch_width = display_width(ch.to_string().as_str(), capabilities).max(1);
        if current_width + ch_width > width {
            break;
        }
        current_width += ch_width;
        out.push(ch);
    }
    out
}

fn display_width(text: &str, capabilities: TerminalCapabilities) -> usize {
    if capabilities.ascii_only() {
        text.chars().count()
    } else {
        UnicodeWidthStr::width(text)
    }
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

fn status_suffix(status: TranscriptCellStatus) -> &'static str {
    match status {
        TranscriptCellStatus::Streaming => " · streaming",
        TranscriptCellStatus::Complete => "",
        TranscriptCellStatus::Failed => " · failed",
        TranscriptCellStatus::Cancelled => " · cancelled",
    }
}
