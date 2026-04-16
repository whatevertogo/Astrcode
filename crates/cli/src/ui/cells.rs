use unicode_width::UnicodeWidthStr;

use super::theme::ThemePalette;
use crate::{
    capability::TerminalCapabilities,
    state::{
        ThinkingPresentationState, TranscriptCell, TranscriptCellKind, TranscriptCellStatus,
        WrappedLine, WrappedLineStyle,
    },
};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TranscriptCellView {
    pub selected: bool,
    pub expanded: bool,
    pub thinking: Option<ThinkingPresentationState>,
}

pub trait RenderableCell {
    fn render_lines(
        &self,
        width: usize,
        capabilities: TerminalCapabilities,
        _theme: &dyn ThemePalette,
        view: &TranscriptCellView,
    ) -> Vec<WrappedLine>;
}

impl RenderableCell for TranscriptCell {
    fn render_lines(
        &self,
        width: usize,
        capabilities: TerminalCapabilities,
        _theme: &dyn ThemePalette,
        view: &TranscriptCellView,
    ) -> Vec<WrappedLine> {
        let width = width.max(28);
        match &self.kind {
            TranscriptCellKind::User { body } => {
                render_message(body, width, capabilities, view, true)
            },
            TranscriptCellKind::Assistant { body, status } => render_message(
                format!("Astrcode{} {}", status_suffix(*status), body).trim(),
                width,
                capabilities,
                view,
                false,
            ),
            TranscriptCellKind::Thinking { .. } => render_thinking_cell(width, capabilities, view),
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
            } => render_tool_call_cell(
                tool_name,
                summary,
                *status,
                stdout,
                stderr,
                error.as_deref(),
                *duration_ms,
                *truncated,
                child_session_id.as_deref(),
                width,
                capabilities,
                view,
            ),
            TranscriptCellKind::Error { code, message } => render_secondary_line(
                &format!("{code} {message}"),
                width,
                capabilities,
                view,
                WrappedLineStyle::ErrorText,
            ),
            TranscriptCellKind::SystemNote { markdown, .. } => {
                render_secondary_line(markdown, width, capabilities, view, WrappedLineStyle::Muted)
            },
            TranscriptCellKind::ChildHandoff { title, message, .. } => render_secondary_line(
                &format!("{title} · {message}"),
                width,
                capabilities,
                view,
                WrappedLineStyle::Muted,
            ),
        }
    }
}

fn render_message(
    body: &str,
    width: usize,
    capabilities: TerminalCapabilities,
    view: &TranscriptCellView,
    is_user: bool,
) -> Vec<WrappedLine> {
    let wrapped = wrap_text(body, width.saturating_sub(4), capabilities);
    let mut lines = Vec::new();
    for (index, line) in wrapped.into_iter().enumerate() {
        let prefix = if index == 0 {
            if is_user {
                prompt_marker(capabilities)
            } else {
                assistant_marker(capabilities)
            }
        } else {
            " "
        };
        lines.push(WrappedLine {
            style: if view.selected {
                WrappedLineStyle::Selection
            } else if is_user && index == 0 {
                WrappedLineStyle::UserLabel
            } else if is_user {
                WrappedLineStyle::UserBody
            } else if index == 0 {
                WrappedLineStyle::AssistantLabel
            } else {
                WrappedLineStyle::AssistantBody
            },
            content: format!("{prefix} {line}"),
        });
    }
    lines.push(blank_line());
    lines
}

fn render_thinking_cell(
    width: usize,
    capabilities: TerminalCapabilities,
    view: &TranscriptCellView,
) -> Vec<WrappedLine> {
    let Some(thinking) = &view.thinking else {
        return Vec::new();
    };
    if !view.expanded {
        return vec![
            WrappedLine {
                style: if view.selected {
                    WrappedLineStyle::Selection
                } else {
                    WrappedLineStyle::ThinkingLabel
                },
                content: truncate_with_ellipsis(
                    format!(
                        "{} {} {}",
                        thinking_marker(capabilities),
                        thinking.label,
                        thinking.preview
                    )
                    .as_str(),
                    width,
                ),
            },
            blank_line(),
        ];
    }

    let mut lines = vec![WrappedLine {
        style: if view.selected {
            WrappedLineStyle::Selection
        } else {
            WrappedLineStyle::ThinkingLabel
        },
        content: format!("{} {}", thinking_marker(capabilities), thinking.label),
    }];
    for line in wrap_text(
        thinking.expanded_body.as_str(),
        width.saturating_sub(2),
        capabilities,
    ) {
        lines.push(WrappedLine {
            style: if view.selected {
                WrappedLineStyle::Selection
            } else {
                WrappedLineStyle::ThinkingBody
            },
            content: format!("  {line}"),
        });
    }
    lines.push(blank_line());
    lines
}

#[allow(clippy::too_many_arguments)]
fn render_tool_call_cell(
    tool_name: &str,
    summary: &str,
    status: TranscriptCellStatus,
    stdout: &str,
    stderr: &str,
    error: Option<&str>,
    duration_ms: Option<u64>,
    truncated: bool,
    child_session_id: Option<&str>,
    width: usize,
    capabilities: TerminalCapabilities,
    view: &TranscriptCellView,
) -> Vec<WrappedLine> {
    let mut lines = vec![WrappedLine {
        style: if view.selected {
            WrappedLineStyle::Selection
        } else {
            WrappedLineStyle::ToolLabel
        },
        content: truncate_with_ellipsis(
            format!(
                "{} tool {}{} · {}",
                tool_marker(capabilities),
                tool_name,
                status_suffix(status),
                summary.trim()
            )
            .as_str(),
            width,
        ),
    }];

    if view.expanded {
        let mut sections = Vec::new();
        if let Some(duration_ms) = duration_ms {
            sections.push(format!("duration {duration_ms}ms"));
        }
        if truncated {
            sections.push("output truncated".to_string());
        }
        if let Some(child_session_id) = child_session_id.filter(|value| !value.is_empty()) {
            sections.push(format!("child session {child_session_id}"));
        }
        if !stdout.trim().is_empty() {
            sections.push(format!("stdout\n{}", stdout.trim_end()));
        }
        if !stderr.trim().is_empty() {
            sections.push(format!("stderr\n{}", stderr.trim_end()));
        }
        if let Some(error) = error.filter(|value| !value.trim().is_empty()) {
            sections.push(format!("error\n{}", error.trim()));
        }
        for line in wrap_text(
            sections.join("\n\n").as_str(),
            width.saturating_sub(2),
            capabilities,
        ) {
            lines.push(WrappedLine {
                style: if view.selected {
                    WrappedLineStyle::Selection
                } else {
                    WrappedLineStyle::ToolBody
                },
                content: format!("  {line}"),
            });
        }
    }

    lines.push(blank_line());
    lines
}

fn render_secondary_line(
    body: &str,
    width: usize,
    capabilities: TerminalCapabilities,
    view: &TranscriptCellView,
    style: WrappedLineStyle,
) -> Vec<WrappedLine> {
    let mut lines = Vec::new();
    for line in wrap_text(body, width.saturating_sub(2), capabilities) {
        lines.push(WrappedLine {
            style: if view.selected {
                WrappedLineStyle::Selection
            } else {
                style
            },
            content: format!("{} {line}", secondary_marker(capabilities)),
        });
    }
    lines.push(blank_line());
    lines
}

fn prompt_marker(capabilities: TerminalCapabilities) -> &'static str {
    if capabilities.ascii_only() {
        ">"
    } else {
        "›"
    }
}

fn assistant_marker(capabilities: TerminalCapabilities) -> &'static str {
    if capabilities.ascii_only() {
        "*"
    } else {
        "•"
    }
}

fn thinking_marker(capabilities: TerminalCapabilities) -> &'static str {
    if capabilities.ascii_only() {
        "*"
    } else {
        "✻"
    }
}

fn tool_marker(capabilities: TerminalCapabilities) -> &'static str {
    if capabilities.ascii_only() {
        "-"
    } else {
        "↳"
    }
}

fn secondary_marker(capabilities: TerminalCapabilities) -> &'static str {
    if capabilities.ascii_only() { "-" } else { "·" }
}

fn blank_line() -> WrappedLine {
    WrappedLine {
        style: WrappedLineStyle::Plain,
        content: String::new(),
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

fn truncate_with_ellipsis(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    if width <= 1 {
        return "…".to_string();
    }
    let mut value = text.chars().take(width - 1).collect::<String>();
    value.push('…');
    value
}

pub fn wrap_text(text: &str, width: usize, capabilities: TerminalCapabilities) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            let next = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            let fits = if capabilities.ascii_only() {
                next.len() <= width
            } else {
                UnicodeWidthStr::width(next.as_str()) <= width
            };
            if fits || current.is_empty() {
                current = next;
            } else {
                lines.push(current);
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}
