use std::borrow::Cow;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{theme::ThemePalette, truncate_to_width};
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
        theme: &dyn ThemePalette,
        view: &TranscriptCellView,
    ) -> Vec<WrappedLine>;
}

impl RenderableCell for TranscriptCell {
    fn render_lines(
        &self,
        width: usize,
        capabilities: TerminalCapabilities,
        theme: &dyn ThemePalette,
        view: &TranscriptCellView,
    ) -> Vec<WrappedLine> {
        let width = width.max(28);
        match &self.kind {
            TranscriptCellKind::User { body } => {
                render_message(body, width, capabilities, theme, view, true)
            },
            TranscriptCellKind::Assistant { body, status } => {
                let content = if matches!(status, TranscriptCellStatus::Streaming) {
                    format!("{body}{}", status_suffix(*status))
                } else {
                    body.clone()
                };
                render_message(content.as_str(), width, capabilities, theme, view, false)
            },
            TranscriptCellKind::Thinking { .. } => {
                render_thinking_cell(width, capabilities, theme, view)
            },
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
                ToolCallView {
                    tool_name,
                    summary,
                    status: *status,
                    stdout,
                    stderr,
                    error: error.as_deref(),
                    duration_ms: *duration_ms,
                    truncated: *truncated,
                    child_session_id: child_session_id.as_deref(),
                },
                width,
                capabilities,
                theme,
                view,
            ),
            TranscriptCellKind::Error { code, message } => render_secondary_line(
                &format!("{code} {message}"),
                width,
                capabilities,
                theme,
                view,
                WrappedLineStyle::ErrorText,
                MarkdownRenderMode::Literal,
            ),
            TranscriptCellKind::SystemNote { markdown, .. } => render_secondary_line(
                markdown,
                width,
                capabilities,
                theme,
                view,
                WrappedLineStyle::Notice,
                MarkdownRenderMode::Display,
            ),
            TranscriptCellKind::ChildHandoff { title, message, .. } => render_secondary_line(
                &format!("{title} · {message}"),
                width,
                capabilities,
                theme,
                view,
                WrappedLineStyle::Notice,
                MarkdownRenderMode::Literal,
            ),
        }
    }
}

impl TranscriptCellView {
    fn resolve_style(&self, base: WrappedLineStyle) -> WrappedLineStyle {
        if self.selected {
            WrappedLineStyle::Selection
        } else {
            base
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkdownRenderMode {
    Literal,
    Display,
}

fn render_message(
    body: &str,
    width: usize,
    capabilities: TerminalCapabilities,
    theme: &dyn ThemePalette,
    view: &TranscriptCellView,
    is_user: bool,
) -> Vec<WrappedLine> {
    let first_prefix = format!(
        "{} ",
        if is_user {
            prompt_marker(theme)
        } else {
            assistant_marker(theme)
        }
    );
    let subsequent_prefix = " ".repeat(display_width(first_prefix.as_str()));
    let wrapped = if is_user {
        wrap_literal_text(
            body,
            width.saturating_sub(display_width(first_prefix.as_str())),
            capabilities,
        )
    } else {
        wrap_text(
            body,
            width.saturating_sub(display_width(first_prefix.as_str())),
            capabilities,
        )
    };
    let style = view.resolve_style(if is_user {
        WrappedLineStyle::PromptEcho
    } else {
        WrappedLineStyle::Plain
    });
    let mut lines = Vec::new();
    for (index, line) in wrapped.into_iter().enumerate() {
        lines.push(WrappedLine {
            style,
            content: format!(
                "{}{}",
                if index == 0 {
                    first_prefix.as_str()
                } else {
                    subsequent_prefix.as_str()
                },
                line
            ),
        });
    }
    lines.push(blank_line());
    lines
}

fn render_thinking_cell(
    width: usize,
    capabilities: TerminalCapabilities,
    theme: &dyn ThemePalette,
    view: &TranscriptCellView,
) -> Vec<WrappedLine> {
    let Some(thinking) = &view.thinking else {
        return Vec::new();
    };
    if !view.expanded {
        return vec![
            WrappedLine {
                style: view.resolve_style(WrappedLineStyle::ThinkingLabel),
                content: truncate_to_width(
                    format!("{} {}", thinking_marker(theme), thinking.summary).as_str(),
                    width,
                ),
            },
            WrappedLine {
                style: view.resolve_style(WrappedLineStyle::ThinkingPreview),
                content: truncate_to_width(
                    format!("  {} {}", thinking_preview_prefix(theme), thinking.preview).as_str(),
                    width,
                ),
            },
            blank_line(),
        ];
    }

    let mut lines = vec![WrappedLine {
        style: view.resolve_style(WrappedLineStyle::ThinkingLabel),
        content: format!("{} {}", thinking_marker(theme), thinking.summary),
    }];
    lines.push(WrappedLine {
        style: view.resolve_style(WrappedLineStyle::ThinkingPreview),
        content: format!("  {}", thinking.hint),
    });
    for line in wrap_text(
        thinking.expanded_body.as_str(),
        width.saturating_sub(2),
        capabilities,
    ) {
        lines.push(WrappedLine {
            style: view.resolve_style(WrappedLineStyle::ThinkingBody),
            content: format!("  {line}"),
        });
    }
    lines.push(blank_line());
    lines
}

struct ToolCallView<'a> {
    tool_name: &'a str,
    summary: &'a str,
    status: TranscriptCellStatus,
    stdout: &'a str,
    stderr: &'a str,
    error: Option<&'a str>,
    duration_ms: Option<u64>,
    truncated: bool,
    child_session_id: Option<&'a str>,
}

fn render_tool_call_cell(
    tool: ToolCallView<'_>,
    width: usize,
    capabilities: TerminalCapabilities,
    theme: &dyn ThemePalette,
    view: &TranscriptCellView,
) -> Vec<WrappedLine> {
    let mut lines = vec![WrappedLine {
        style: view.resolve_style(WrappedLineStyle::ToolLabel),
        content: truncate_to_width(
            format!(
                "{} tool {}{} · {}",
                tool_marker(theme),
                tool.tool_name,
                status_suffix(tool.status),
                tool.summary.trim()
            )
            .as_str(),
            width,
        ),
    }];

    if view.expanded {
        let mut metadata = Vec::new();
        if let Some(duration_ms) = tool.duration_ms {
            metadata.push(format!("duration {duration_ms}ms"));
        }
        if tool.truncated {
            metadata.push("output truncated".to_string());
        }
        if let Some(child_session_id) = tool.child_session_id.filter(|value| !value.is_empty()) {
            metadata.push(format!("child session {child_session_id}"));
        }
        if !metadata.is_empty() {
            lines.push(WrappedLine {
                style: view.resolve_style(WrappedLineStyle::ToolBody),
                content: format!("  meta {}", metadata.join(" · ")),
            });
        }

        if !tool.stdout.trim().is_empty() {
            append_preformatted_tool_section(
                &mut lines,
                "stdout",
                tool.stdout.trim_end(),
                width,
                capabilities,
                theme,
                view,
            );
        }

        if !tool.stderr.trim().is_empty() {
            append_preformatted_tool_section(
                &mut lines,
                "stderr",
                tool.stderr.trim_end(),
                width,
                capabilities,
                theme,
                view,
            );
        }

        if let Some(error) = tool.error.filter(|value| !value.trim().is_empty()) {
            append_preformatted_tool_section(
                &mut lines,
                "error",
                error.trim(),
                width,
                capabilities,
                theme,
                view,
            );
        }
    }

    lines.push(blank_line());
    lines
}

fn append_preformatted_tool_section(
    lines: &mut Vec<WrappedLine>,
    label: &str,
    body: &str,
    width: usize,
    capabilities: TerminalCapabilities,
    theme: &dyn ThemePalette,
    view: &TranscriptCellView,
) {
    let section_style = view.resolve_style(WrappedLineStyle::ToolBody);
    lines.push(WrappedLine {
        style: section_style,
        content: format!("  {label}"),
    });
    for line in render_preformatted_block(body, width.saturating_sub(4), capabilities) {
        lines.push(WrappedLine {
            style: section_style,
            content: format!("  {} {line}", tool_block_marker(theme)),
        });
    }
}

fn render_secondary_line(
    body: &str,
    width: usize,
    capabilities: TerminalCapabilities,
    theme: &dyn ThemePalette,
    view: &TranscriptCellView,
    style: WrappedLineStyle,
    render_mode: MarkdownRenderMode,
) -> Vec<WrappedLine> {
    let mut lines = Vec::new();
    for line in wrap_text_with_mode(body, width.saturating_sub(2), capabilities, render_mode) {
        lines.push(WrappedLine {
            style: view.resolve_style(style),
            content: format!("{} {line}", secondary_marker(theme)),
        });
    }
    lines.push(blank_line());
    lines
}

fn prompt_marker(theme: &dyn ThemePalette) -> &'static str {
    theme.glyph("›", ">")
}

fn assistant_marker(theme: &dyn ThemePalette) -> &'static str {
    theme.glyph("•", "*")
}

fn thinking_marker(theme: &dyn ThemePalette) -> &'static str {
    theme.glyph("∴", "~")
}

fn thinking_preview_prefix(theme: &dyn ThemePalette) -> &'static str {
    theme.glyph("└", "|")
}

fn tool_marker(theme: &dyn ThemePalette) -> &'static str {
    theme.glyph("↳", "=")
}

fn secondary_marker(theme: &dyn ThemePalette) -> &'static str {
    theme.glyph("·", "-")
}

fn tool_block_marker(theme: &dyn ThemePalette) -> &'static str {
    theme.glyph("│", "|")
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

pub fn wrap_text(text: &str, width: usize, capabilities: TerminalCapabilities) -> Vec<String> {
    wrap_text_with_mode(text, width, capabilities, MarkdownRenderMode::Display)
}

fn wrap_literal_text(text: &str, width: usize, capabilities: TerminalCapabilities) -> Vec<String> {
    wrap_text_with_mode(text, width, capabilities, MarkdownRenderMode::Literal)
}

fn wrap_text_with_mode(
    text: &str,
    width: usize,
    capabilities: TerminalCapabilities,
    render_mode: MarkdownRenderMode,
) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut output = Vec::new();
    let source_lines = text.lines().collect::<Vec<_>>();
    let mut index = 0;
    let mut in_fence = false;
    let mut fence_marker = "";

    while index < source_lines.len() {
        let line = source_lines[index];
        let trimmed = line.trim_end();

        if trimmed.is_empty() {
            output.push(String::new());
            index += 1;
            continue;
        }

        if matches!(render_mode, MarkdownRenderMode::Display) {
            if is_horizontal_rule(trimmed) {
                output.push(render_horizontal_rule(width, capabilities));
                index += 1;
                continue;
            }

            if let Some((level, heading)) = parse_heading(trimmed) {
                let heading = normalize_inline_markdown(heading, render_mode);
                output.extend(render_heading(level, heading.as_str(), width, capabilities));
                index += 1;
                continue;
            }
        }

        if let Some(marker) = fence_delimiter(trimmed) {
            in_fence = !in_fence;
            fence_marker = if in_fence { marker } else { "" };
            output.extend(wrap_preformatted_line(trimmed, width, capabilities));
            index += 1;
            continue;
        }

        if in_fence {
            if !fence_marker.is_empty() && trimmed.trim_start().starts_with(fence_marker) {
                in_fence = false;
                fence_marker = "";
            }
            output.extend(wrap_preformatted_line(trimmed, width, capabilities));
            index += 1;
            continue;
        }

        if is_table_line(trimmed) {
            let mut block = Vec::new();
            while index < source_lines.len() && is_table_line(source_lines[index].trim_end()) {
                block.push(source_lines[index].trim_end());
                index += 1;
            }
            output.extend(render_table_block(&block, width, capabilities, render_mode));
            continue;
        }

        if let Some((prefix, body)) = parse_list_prefix(trimmed) {
            let body = normalize_inline_markdown(body, render_mode);
            output.extend(wrap_with_prefix(
                body.as_str(),
                width,
                capabilities,
                &prefix,
                &indent_like(&prefix),
            ));
            index += 1;
            continue;
        }

        if let Some((prefix, body)) = parse_quote_prefix(trimmed) {
            let body = normalize_inline_markdown(body, render_mode);
            output.extend(wrap_with_prefix(
                body.as_str(),
                width,
                capabilities,
                &prefix,
                &indent_like(&prefix),
            ));
            index += 1;
            continue;
        }

        if is_preformatted_line(line) {
            output.extend(wrap_preformatted_line(trimmed, width, capabilities));
            index += 1;
            continue;
        }

        let mut paragraph = vec![trimmed.trim()];
        index += 1;
        while index < source_lines.len() {
            let next = source_lines[index].trim_end();
            if next.is_empty()
                || fence_delimiter(next).is_some()
                || is_table_line(next)
                || parse_list_prefix(next).is_some()
                || parse_quote_prefix(next).is_some()
                || is_preformatted_line(source_lines[index])
            {
                break;
            }
            paragraph.push(next.trim());
            index += 1;
        }
        let paragraph = normalize_inline_markdown(paragraph.join(" ").as_str(), render_mode);
        output.extend(wrap_paragraph(paragraph.as_str(), width, capabilities));
    }

    if output.is_empty() {
        output.push(String::new());
    }
    output
}

fn fence_delimiter(line: &str) -> Option<&'static str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("```") {
        Some("```")
    } else if trimmed.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

fn is_table_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with('|') && trimmed.ends_with('|') && trimmed.matches('|').count() >= 2
}

fn is_preformatted_line(line: &str) -> bool {
    line.starts_with("    ") || line.starts_with('\t')
}

fn parse_list_prefix(line: &str) -> Option<(String, &str)> {
    let indent_width = line.chars().take_while(|ch| ch.is_whitespace()).count();
    let trimmed = line.trim_start();

    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(marker) {
            return Some((
                format!("{}{}", " ".repeat(indent_width), marker),
                rest.trim_start(),
            ));
        }
    }

    let digits = trimmed
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    let remainder = &trimmed[digits.len()..];
    for punct in [". ", ") "] {
        if let Some(rest) = remainder.strip_prefix(punct) {
            return Some((
                format!("{}{}{}", " ".repeat(indent_width), digits, punct),
                rest.trim_start(),
            ));
        }
    }
    None
}

fn parse_quote_prefix(line: &str) -> Option<(String, &str)> {
    let indent_width = line.chars().take_while(|ch| ch.is_whitespace()).count();
    let trimmed = line.trim_start();
    trimmed
        .strip_prefix("> ")
        .map(|rest| (format!("{}> ", " ".repeat(indent_width)), rest.trim_start()))
}

fn parse_heading(line: &str) -> Option<(usize, &str)> {
    let trimmed = line.trim();
    let hashes = trimmed.bytes().take_while(|byte| *byte == b'#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let body = trimmed.get(hashes..)?.strip_prefix(' ')?;
    Some((hashes, body.trim_end().trim_end_matches('#').trim_end()))
}

fn is_horizontal_rule(line: &str) -> bool {
    let compact = line
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    if compact.len() < 3 {
        return false;
    }
    let mut chars = compact.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    matches!(first, '-' | '*' | '_') && chars.all(|ch| ch == first)
}

fn render_horizontal_rule(width: usize, capabilities: TerminalCapabilities) -> String {
    let glyph = if capabilities.ascii_only() {
        "-"
    } else {
        "─"
    };
    glyph.repeat(width.clamp(3, 48))
}

fn render_heading(
    level: usize,
    body: &str,
    width: usize,
    capabilities: TerminalCapabilities,
) -> Vec<String> {
    let normalized = body.trim();
    if normalized.is_empty() {
        return vec![String::new()];
    }

    let underline_glyph = match (level, capabilities.ascii_only()) {
        (1, false) => "═",
        (1, true) => "=",
        (_, false) => "─",
        (_, true) => "-",
    };
    let heading_width = display_width(normalized).clamp(3, width.max(3).min(48));
    let mut lines = wrap_paragraph(normalized, width, capabilities);
    if level <= 2 {
        lines.push(underline_glyph.repeat(heading_width));
    }
    lines
}

fn normalize_inline_markdown(text: &str, render_mode: MarkdownRenderMode) -> String {
    match render_mode {
        MarkdownRenderMode::Literal => text.to_string(),
        MarkdownRenderMode::Display => render_inline_markdown(text),
    }
}

fn render_inline_markdown(text: &str) -> String {
    let chars = text.chars().collect::<Vec<_>>();
    let mut output = String::new();
    let mut index = 0;

    while index < chars.len() {
        if chars[index] == '\\' {
            if let Some(next) = chars.get(index + 1) {
                output.push(*next);
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }

        if let Some((rendered, next)) = parse_inline_code(&chars, index) {
            output.push_str(rendered.as_str());
            index = next;
            continue;
        }

        if let Some((rendered, next)) = parse_link_or_image(&chars, index) {
            output.push_str(rendered.as_str());
            index = next;
            continue;
        }

        if let Some((rendered, next)) = parse_autolink(&chars, index) {
            output.push_str(rendered.as_str());
            index = next;
            continue;
        }

        if let Some((rendered, next)) = parse_delimited_span(&chars, index, "**") {
            output.push_str(rendered.as_str());
            index = next;
            continue;
        }

        if let Some((rendered, next)) = parse_delimited_span(&chars, index, "~~") {
            output.push_str(rendered.as_str());
            index = next;
            continue;
        }

        if let Some((rendered, next)) = parse_delimited_span(&chars, index, "*") {
            output.push_str(rendered.as_str());
            index = next;
            continue;
        }

        output.push(chars[index]);
        index += 1;
    }

    output
}

fn parse_inline_code(chars: &[char], index: usize) -> Option<(String, usize)> {
    if chars.get(index) != Some(&'`') {
        return None;
    }
    let closing = chars[index + 1..]
        .iter()
        .position(|ch| *ch == '`')
        .map(|offset| index + 1 + offset)?;
    Some((
        chars[index + 1..closing].iter().collect::<String>(),
        closing + 1,
    ))
}

fn parse_link_or_image(chars: &[char], index: usize) -> Option<(String, usize)> {
    let is_image = chars.get(index) == Some(&'!') && chars.get(index + 1) == Some(&'[');
    let label_start = if is_image {
        index + 1
    } else if chars.get(index) == Some(&'[') {
        index
    } else {
        return None;
    };

    let (label, next_index) = parse_bracketed(chars, label_start, '[', ']')?;
    if chars.get(next_index) != Some(&'(') {
        return None;
    }
    let (destination, end_index) = parse_bracketed(chars, next_index, '(', ')')?;

    let label = render_inline_markdown(label.as_str());
    let destination = destination.trim().to_string();

    let rendered = if is_image {
        if label.is_empty() {
            "[image]".to_string()
        } else {
            label
        }
    } else if destination.is_empty() || label.is_empty() || label == destination {
        label
    } else {
        format!("{label} ({destination})")
    };

    Some((rendered, end_index))
}

fn parse_autolink(chars: &[char], index: usize) -> Option<(String, usize)> {
    if chars.get(index) != Some(&'<') {
        return None;
    }
    let closing = chars[index + 1..]
        .iter()
        .position(|ch| *ch == '>')
        .map(|offset| index + 1 + offset)?;
    let body = chars[index + 1..closing].iter().collect::<String>();
    if body.contains("://") || body.contains('@') {
        Some((body, closing + 1))
    } else {
        None
    }
}

fn parse_delimited_span(chars: &[char], index: usize, marker: &str) -> Option<(String, usize)> {
    if !matches_marker(chars, index, marker) || !emphasis_can_open(chars, index, marker) {
        return None;
    }
    let marker_width = marker.chars().count();
    let mut cursor = index + marker_width;
    while cursor < chars.len() {
        if matches_marker(chars, cursor, marker) && emphasis_can_close(chars, cursor, marker) {
            let inner = chars[index + marker_width..cursor]
                .iter()
                .collect::<String>();
            return Some((
                render_inline_markdown(inner.as_str()),
                cursor + marker_width,
            ));
        }
        cursor += 1;
    }
    None
}

fn matches_marker(chars: &[char], index: usize, marker: &str) -> bool {
    for (offset, expected) in marker.chars().enumerate() {
        if chars.get(index + offset) != Some(&expected) {
            return false;
        }
    }
    true
}

fn emphasis_can_open(chars: &[char], index: usize, marker: &str) -> bool {
    let marker_width = marker.chars().count();
    let prev = index
        .checked_sub(1)
        .and_then(|position| chars.get(position));
    let next = chars.get(index + marker_width);
    next.is_some_and(|ch| !ch.is_whitespace()) && prev.is_none_or(|ch| emphasis_boundary(*ch))
}

fn emphasis_can_close(chars: &[char], index: usize, marker: &str) -> bool {
    let marker_width = marker.chars().count();
    let prev = index
        .checked_sub(1)
        .and_then(|position| chars.get(position));
    let next = chars.get(index + marker_width);
    prev.is_some_and(|ch| !ch.is_whitespace()) && next.is_none_or(|ch| emphasis_boundary(*ch))
}

fn emphasis_boundary(ch: char) -> bool {
    !ch.is_alphanumeric()
}

fn parse_bracketed(
    chars: &[char],
    index: usize,
    open: char,
    close: char,
) -> Option<(String, usize)> {
    if chars.get(index) != Some(&open) {
        return None;
    }

    let mut cursor = index + 1;
    let mut body = String::new();
    while cursor < chars.len() {
        match chars[cursor] {
            '\\' => {
                if let Some(next) = chars.get(cursor + 1) {
                    body.push(*next);
                    cursor += 2;
                } else {
                    cursor += 1;
                }
            },
            ch if ch == close => return Some((body, cursor + 1)),
            ch => {
                body.push(ch);
                cursor += 1;
            },
        }
    }
    None
}

fn indent_like(prefix: &str) -> String {
    " ".repeat(display_width(prefix))
}

fn wrap_paragraph(text: &str, width: usize, capabilities: TerminalCapabilities) -> Vec<String> {
    wrap_with_prefix(text, width, capabilities, "", "")
}

fn wrap_with_prefix(
    text: &str,
    width: usize,
    capabilities: TerminalCapabilities,
    first_prefix: &str,
    subsequent_prefix: &str,
) -> Vec<String> {
    let mut lines = Vec::new();
    let first_prefix_width = display_width(first_prefix);
    let subsequent_prefix_width = display_width(subsequent_prefix);
    let first_available = width.saturating_sub(first_prefix_width).max(1);
    let subsequent_available = width.saturating_sub(subsequent_prefix_width).max(1);

    let mut current = first_prefix.to_string();
    let mut current_width = first_prefix_width;
    let mut current_prefix = first_prefix;
    let mut current_available = first_available;

    for token in text.split_whitespace() {
        for chunk in split_token_by_width(token, current_available.max(1), capabilities) {
            let chunk_width = display_width(chunk.as_ref());
            let needs_space = current_width > display_width(current_prefix);
            let next_width = current_width + usize::from(needs_space) + chunk_width;
            if next_width <= width {
                if needs_space {
                    current.push(' ');
                    current_width += 1;
                }
                current.push_str(chunk.as_ref());
                current_width += chunk_width;
                continue;
            }

            if current_width > display_width(current_prefix) {
                lines.push(current);
                current = subsequent_prefix.to_string();
                current_width = subsequent_prefix_width;
                current_prefix = subsequent_prefix;
                current_available = subsequent_available;
            }

            if current_width == display_width(current_prefix) {
                current.push_str(chunk.as_ref());
                current_width += chunk_width;
            }
        }
    }

    if current_width > display_width(current_prefix) || lines.is_empty() {
        lines.push(current);
    }
    lines
}

fn wrap_preformatted_line(
    line: &str,
    width: usize,
    capabilities: TerminalCapabilities,
) -> Vec<String> {
    let indent = line
        .chars()
        .take_while(|ch| ch.is_whitespace())
        .collect::<String>();
    let content = line[indent.len()..].trim_end();
    let prefix_width = display_width(indent.as_str());
    let available = width.saturating_sub(prefix_width).max(1);
    let chunks = split_preserving_width(content, available, capabilities);
    if chunks.is_empty() {
        return vec![indent];
    }
    chunks
        .into_iter()
        .map(|chunk| format!("{indent}{chunk}"))
        .collect()
}

fn render_preformatted_block(
    body: &str,
    width: usize,
    capabilities: TerminalCapabilities,
) -> Vec<String> {
    let mut lines = Vec::new();
    let source_lines = body.lines().collect::<Vec<_>>();
    let mut index = 0;
    while index < source_lines.len() {
        let line = source_lines[index].trim_end();
        if is_table_line(line) {
            let mut block = Vec::new();
            while index < source_lines.len() && is_table_line(source_lines[index].trim_end()) {
                block.push(source_lines[index].trim_end());
                index += 1;
            }
            lines.extend(render_table_block(
                &block,
                width,
                capabilities,
                MarkdownRenderMode::Literal,
            ));
            continue;
        }
        lines.extend(wrap_preformatted_line(line, width, capabilities));
        index += 1;
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn render_table_block(
    lines: &[&str],
    width: usize,
    capabilities: TerminalCapabilities,
    render_mode: MarkdownRenderMode,
) -> Vec<String> {
    let rows = lines
        .iter()
        .map(|line| parse_table_row(line, render_mode))
        .collect::<Vec<_>>();
    let col_count = rows.iter().map(Vec::len).max().unwrap_or(0);
    if col_count == 0 {
        return lines
            .iter()
            .flat_map(|line| wrap_preformatted_line(line, width, capabilities))
            .collect();
    }

    let separator_rows = rows
        .iter()
        .map(|row| row.iter().all(|cell| is_table_separator(cell.as_str())))
        .collect::<Vec<_>>();
    let mut col_widths = vec![3usize; col_count];
    for (row_index, row) in rows.iter().enumerate() {
        if separator_rows[row_index] {
            continue;
        }
        for (index, cell) in row.iter().enumerate() {
            col_widths[index] = col_widths[index].max(display_width(cell.as_str()).min(40));
        }
    }

    let min_widths = vec![3usize; col_count];
    let separator_width = col_count * 3 + 1;
    let max_budget = width.saturating_sub(separator_width);
    while col_widths.iter().sum::<usize>() > max_budget {
        let Some((index, _)) = col_widths
            .iter()
            .enumerate()
            .filter(|(index, value)| **value > min_widths[*index])
            .max_by_key(|(_, value)| **value)
        else {
            break;
        };
        col_widths[index] = col_widths[index].saturating_sub(1);
    }

    if matches!(render_mode, MarkdownRenderMode::Literal) {
        return rows
            .iter()
            .enumerate()
            .map(|(row_index, row)| {
                if separator_rows[row_index] {
                    render_plain_table_separator(&col_widths)
                } else {
                    render_plain_table_row(row, &col_widths)
                }
            })
            .collect();
    }

    render_boxed_table(rows, separator_rows, col_widths, capabilities)
}

fn parse_table_row(line: &str, render_mode: MarkdownRenderMode) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| normalize_inline_markdown(cell.trim(), render_mode))
        .collect()
}

fn is_table_separator(cell: &str) -> bool {
    let trimmed = cell.trim();
    !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|ch| ch == '-' || ch == ':' || ch.is_whitespace())
}

fn render_plain_table_separator(col_widths: &[usize]) -> String {
    let mut line = String::from("|");
    for width in col_widths {
        line.push_str(&"-".repeat(width.saturating_add(2)));
        line.push('|');
    }
    line
}

fn render_plain_table_row(row: &[String], col_widths: &[usize]) -> String {
    let mut line = String::from("|");
    for (index, width) in col_widths.iter().enumerate() {
        let cell = row.get(index).map(String::as_str).unwrap_or("");
        line.push(' ');
        line.push_str(pad_to_width(truncate_to_width(cell, *width).as_str(), *width).as_str());
        line.push(' ');
        line.push('|');
    }
    line
}

fn render_boxed_table(
    rows: Vec<Vec<String>>,
    separator_rows: Vec<bool>,
    col_widths: Vec<usize>,
    capabilities: TerminalCapabilities,
) -> Vec<String> {
    let chars = table_chars(capabilities);
    let mut rendered = Vec::new();
    let mut emitted_top = false;
    let mut saw_header_separator = false;
    let has_separator_row = separator_rows.iter().any(|is_separator| *is_separator);

    for (row_index, row) in rows.iter().enumerate() {
        if separator_rows[row_index] {
            if !emitted_top {
                rendered.push(render_table_border(
                    &col_widths,
                    chars.top_left,
                    chars.top_mid,
                    chars.top_right,
                    chars.horizontal,
                ));
                emitted_top = true;
            }
            rendered.push(render_table_border(
                &col_widths,
                chars.mid_left,
                chars.mid_mid,
                chars.mid_right,
                chars.horizontal,
            ));
            saw_header_separator = true;
            continue;
        }

        if !emitted_top {
            rendered.push(render_table_border(
                &col_widths,
                chars.top_left,
                chars.top_mid,
                chars.top_right,
                chars.horizontal,
            ));
            emitted_top = true;
        }

        rendered.push(render_boxed_table_row(row, &col_widths, chars.vertical));

        if !has_separator_row && row_index + 1 < rows.len() {
            rendered.push(render_table_border(
                &col_widths,
                chars.mid_left,
                chars.mid_mid,
                chars.mid_right,
                chars.horizontal,
            ));
        } else if has_separator_row
            && saw_header_separator
            && row_index + 1 < rows.len()
            && separator_rows.get(row_index + 1).copied().unwrap_or(false)
        {
            rendered.push(render_table_border(
                &col_widths,
                chars.mid_left,
                chars.mid_mid,
                chars.mid_right,
                chars.horizontal,
            ));
        }
    }

    if emitted_top {
        rendered.push(render_table_border(
            &col_widths,
            chars.bottom_left,
            chars.bottom_mid,
            chars.bottom_right,
            chars.horizontal,
        ));
    }

    rendered
}

fn render_table_border(
    col_widths: &[usize],
    left: &str,
    middle: &str,
    right: &str,
    horizontal: &str,
) -> String {
    let mut line = String::from(left);
    for (index, width) in col_widths.iter().enumerate() {
        line.push_str(&horizontal.repeat(width.saturating_add(2)));
        if index + 1 == col_widths.len() {
            line.push_str(right);
        } else {
            line.push_str(middle);
        }
    }
    line
}

fn render_boxed_table_row(row: &[String], col_widths: &[usize], vertical: &str) -> String {
    let mut line = String::from(vertical);
    for (index, width) in col_widths.iter().enumerate() {
        let cell = row.get(index).map(String::as_str).unwrap_or("");
        line.push(' ');
        line.push_str(pad_to_width(truncate_to_width(cell, *width).as_str(), *width).as_str());
        line.push(' ');
        line.push_str(vertical);
    }
    line
}

#[derive(Debug, Clone, Copy)]
struct TableChars<'a> {
    top_left: &'a str,
    top_mid: &'a str,
    top_right: &'a str,
    mid_left: &'a str,
    mid_mid: &'a str,
    mid_right: &'a str,
    bottom_left: &'a str,
    bottom_mid: &'a str,
    bottom_right: &'a str,
    horizontal: &'a str,
    vertical: &'a str,
}

fn table_chars(capabilities: TerminalCapabilities) -> TableChars<'static> {
    if capabilities.ascii_only() {
        TableChars {
            top_left: "+",
            top_mid: "+",
            top_right: "+",
            mid_left: "+",
            mid_mid: "+",
            mid_right: "+",
            bottom_left: "+",
            bottom_mid: "+",
            bottom_right: "+",
            horizontal: "-",
            vertical: "|",
        }
    } else {
        TableChars {
            top_left: "┌",
            top_mid: "┬",
            top_right: "┐",
            mid_left: "├",
            mid_mid: "┼",
            mid_right: "┤",
            bottom_left: "└",
            bottom_mid: "┴",
            bottom_right: "┘",
            horizontal: "─",
            vertical: "│",
        }
    }
}

fn split_token_by_width<'a>(
    token: &'a str,
    width: usize,
    capabilities: TerminalCapabilities,
) -> Vec<Cow<'a, str>> {
    let width = width.max(1);
    if display_width(token) <= width {
        return vec![Cow::Borrowed(token)];
    }
    split_preserving_width(token, width, capabilities)
}

fn split_preserving_width<'a>(
    text: &'a str,
    width: usize,
    _capabilities: TerminalCapabilities,
) -> Vec<Cow<'a, str>> {
    let width = width.max(1);
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;

    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0).max(1);
        if current_width + ch_width > width && !current.is_empty() {
            chunks.push(Cow::Owned(current));
            current = String::new();
            current_width = 0;
        }
        current.push(ch);
        current_width += ch_width;
    }

    if !current.is_empty() {
        chunks.push(Cow::Owned(current));
    }
    chunks
}

fn pad_to_width(text: &str, width: usize) -> String {
    let current_width = display_width(text);
    if current_width >= width {
        return text.to_string();
    }
    format!("{text}{}", " ".repeat(width - current_width))
}

fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

#[cfg(test)]
mod tests {
    use super::{
        RenderableCell, TranscriptCellView, assistant_marker, secondary_marker, thinking_marker,
        tool_marker, wrap_literal_text, wrap_text,
    };
    use crate::{
        capability::{ColorLevel, GlyphMode, TerminalCapabilities},
        state::{TranscriptCell, TranscriptCellKind, TranscriptCellStatus},
        ui::CodexTheme,
    };

    fn unicode_capabilities() -> TerminalCapabilities {
        TerminalCapabilities {
            color: ColorLevel::TrueColor,
            glyphs: GlyphMode::Unicode,
            alt_screen: false,
            mouse: false,
            bracketed_paste: false,
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

    #[test]
    fn wrap_text_preserves_hanging_indent_for_lists() {
        let lines = wrap_text(
            "- 第一项需要被正确换行，并且后续行要和正文对齐",
            18,
            unicode_capabilities(),
        );
        assert!(lines[0].starts_with("- "));
        assert!(lines[1].starts_with("  "));
    }

    #[test]
    fn wrap_text_breaks_cjk_without_spaces() {
        let lines = wrap_text(
            "这是一个没有空格但是需要自动换行的长句子",
            10,
            unicode_capabilities(),
        );
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|line| !line.is_empty()));
    }

    #[test]
    fn wrap_text_keeps_last_token_after_soft_wrap() {
        let lines = wrap_text(
            "查看 readFile (https://example.com/read-file) 与 writeFile。",
            48,
            unicode_capabilities(),
        );
        let joined = lines.join("\n");
        assert!(joined.contains("writeFile。"));
    }

    #[test]
    fn wrap_text_formats_markdown_tables() {
        let lines = wrap_text(
            "| 工具 | 说明 |\n| --- | --- |\n| **reviewnow** | 代码审查 |\n| `git-commit` | \
             自动提交 |",
            32,
            unicode_capabilities(),
        );
        assert!(lines.iter().any(|line| line.contains("┌")));
        assert!(lines.iter().any(|line| line.contains("│ 工具")));
        assert!(lines.iter().any(|line| line.contains("reviewnow")));
        assert!(lines.iter().any(|line| line.contains("git-commit")));
        assert!(lines.iter().all(|line| !line.contains("**reviewnow**")));
        assert!(lines.iter().all(|line| !line.contains("`git-commit`")));
        assert!(lines.iter().any(|line| line.contains("└")));
    }

    #[test]
    fn wrap_text_normalizes_headings_and_links() {
        let lines = wrap_text(
            "## 文件操作\n\n查看 [readFile](https://example.com/read-file) 与 **writeFile**。",
            48,
            unicode_capabilities(),
        );
        let joined = lines.join("\n");
        assert!(joined.contains("文件操作"));
        assert!(joined.contains("────"));
        assert!(joined.contains("readFile"));
        assert!(joined.contains("https://example.com/read-file"));
        assert!(joined.contains("writeFile"));
        assert!(!joined.contains("## 文件操作"));
        assert!(!joined.contains("**writeFile**"));
        assert!(!joined.contains("[readFile]"));
    }

    #[test]
    fn inline_markdown_keeps_emphasis_body_before_cjk_punctuation() {
        assert_eq!(
            super::render_inline_markdown("**writeFile**。"),
            "writeFile。"
        );
    }

    #[test]
    fn wrap_literal_text_preserves_user_markdown_markers() {
        let lines = wrap_literal_text(
            "## 用户原文\n请保留 **readFile** 和 [link](https://example.com)。",
            120,
            unicode_capabilities(),
        );
        let joined = lines.join("\n");
        assert!(joined.contains("## 用户原文"));
        assert!(joined.contains("**readFile**"));
        assert!(joined.contains("[link](https://example.com)"));
    }

    #[test]
    fn wrap_literal_text_keeps_plain_markdown_table_shape() {
        let lines = wrap_literal_text(
            "| 工具 | 说明 |\n| --- | --- |\n| readFile | 读取文件 |",
            48,
            unicode_capabilities(),
        );
        let joined = lines.join("\n");
        assert!(joined.contains("| 工具"));
        assert!(joined.contains("---"));
        assert!(joined.contains("readFile"));
        assert!(!joined.contains("┌"));
    }

    #[test]
    fn ascii_markers_remain_distinct_by_cell_type() {
        let theme = CodexTheme::new(ascii_capabilities());
        assert_ne!(assistant_marker(&theme), thinking_marker(&theme));
        assert_ne!(tool_marker(&theme), secondary_marker(&theme));
    }

    #[test]
    fn assistant_wrapped_lines_use_hanging_indent() {
        let theme = CodexTheme::new(unicode_capabilities());
        let cell = TranscriptCell {
            id: "assistant-1".to_string(),
            expanded: false,
            kind: TranscriptCellKind::Assistant {
                body: "你好！我是 AstrCode，你的本地 AI \
                       编码助手。我可以帮你处理代码编写、文件编辑、终端命令、\
                       代码审查等各种开发任务。"
                    .to_string(),
                status: TranscriptCellStatus::Complete,
            },
        };

        let lines = cell.render_lines(
            36,
            unicode_capabilities(),
            &theme,
            &TranscriptCellView::default(),
        );

        assert!(lines.len() >= 3);
        assert!(lines[0].content.starts_with("• "));
        assert!(lines[1].content.starts_with("  "));
        assert!(!lines[1].content.starts_with("   "));
    }

    #[test]
    fn assistant_rendering_preserves_markdown_line_breaks() {
        let theme = CodexTheme::new(unicode_capabilities());
        let cell = TranscriptCell {
            id: "assistant-2".to_string(),
            expanded: false,
            kind: TranscriptCellKind::Assistant {
                body: "你好！\n\n- 第一项\n- 第二项".to_string(),
                status: TranscriptCellStatus::Complete,
            },
        };

        let lines = cell.render_lines(
            36,
            unicode_capabilities(),
            &theme,
            &TranscriptCellView::default(),
        );

        assert!(lines.iter().any(|line| line.content == "  "));
        assert!(lines.iter().any(|line| line.content.contains("- 第一项")));
        assert!(lines.iter().any(|line| line.content.contains("- 第二项")));
    }

    #[test]
    fn assistant_rendering_strips_markdown_syntax_markers() {
        let theme = CodexTheme::new(unicode_capabilities());
        let cell = TranscriptCell {
            id: "assistant-3".to_string(),
            expanded: false,
            kind: TranscriptCellKind::Assistant {
                body: "## 文件操作\n\n| 工具 | 说明 |\n| --- | --- |\n| **readFile** | 读取 \
                       `README.md` |"
                    .to_string(),
                status: TranscriptCellStatus::Complete,
            },
        };

        let lines = cell.render_lines(
            48,
            unicode_capabilities(),
            &theme,
            &TranscriptCellView::default(),
        );
        let rendered = lines
            .iter()
            .map(|line| line.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("文件操作"));
        assert!(rendered.contains("readFile"));
        assert!(rendered.contains("README.md"));
        assert!(!rendered.contains("## 文件操作"));
        assert!(!rendered.contains("**readFile**"));
        assert!(!rendered.contains("`README.md`"));
    }
}
