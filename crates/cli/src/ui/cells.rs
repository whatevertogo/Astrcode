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
            ),
            TranscriptCellKind::SystemNote { markdown, .. } => render_secondary_line(
                markdown,
                width,
                capabilities,
                theme,
                view,
                WrappedLineStyle::Notice,
            ),
            TranscriptCellKind::ChildHandoff { title, message, .. } => render_secondary_line(
                &format!("{title} · {message}"),
                width,
                capabilities,
                theme,
                view,
                WrappedLineStyle::Notice,
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

fn render_message(
    body: &str,
    width: usize,
    capabilities: TerminalCapabilities,
    theme: &dyn ThemePalette,
    view: &TranscriptCellView,
    is_user: bool,
) -> Vec<WrappedLine> {
    let wrapped = wrap_text(body, width.saturating_sub(4), capabilities);
    let mut lines = Vec::new();
    for (index, line) in wrapped.into_iter().enumerate() {
        let prefix = if is_user {
            prompt_marker(theme)
        } else if index == 0 {
            assistant_marker(theme)
        } else {
            "  "
        };
        lines.push(WrappedLine {
            style: view.resolve_style(if is_user {
                WrappedLineStyle::PromptEcho
            } else {
                WrappedLineStyle::Plain
            }),
            content: format!("{prefix} {line}"),
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
) -> Vec<WrappedLine> {
    let mut lines = Vec::new();
    for line in wrap_text(body, width.saturating_sub(2), capabilities) {
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
    theme.glyph("∴", "*")
}

fn thinking_preview_prefix(theme: &dyn ThemePalette) -> &'static str {
    theme.glyph("└", "|")
}

fn tool_marker(theme: &dyn ThemePalette) -> &'static str {
    theme.glyph("↳", "-")
}

fn secondary_marker(theme: &dyn ThemePalette) -> &'static str {
    theme.glyph("·", "-")
}

fn tool_block_marker(theme: &dyn ThemePalette) -> &'static str {
    theme.glyph("│", "|")
}

pub(crate) fn synthetic_thinking_lines(
    theme: &dyn ThemePalette,
    presentation: &ThinkingPresentationState,
) -> Vec<WrappedLine> {
    vec![
        WrappedLine {
            style: WrappedLineStyle::ThinkingLabel,
            content: format!("{} {}", thinking_marker(theme), presentation.summary),
        },
        WrappedLine {
            style: WrappedLineStyle::ThinkingPreview,
            content: format!(
                "  {} {}",
                thinking_preview_prefix(theme),
                presentation.preview
            ),
        },
        WrappedLine {
            style: WrappedLineStyle::ThinkingPreview,
            content: format!("  {}", presentation.hint),
        },
        blank_line(),
    ]
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
            output.extend(render_table_block(&block, width, capabilities));
            continue;
        }

        if let Some((prefix, body)) = parse_list_prefix(trimmed) {
            output.extend(wrap_with_prefix(
                body,
                width,
                capabilities,
                &prefix,
                &indent_like(&prefix),
            ));
            index += 1;
            continue;
        }

        if let Some((prefix, body)) = parse_quote_prefix(trimmed) {
            output.extend(wrap_with_prefix(
                body,
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
        output.extend(wrap_paragraph(
            paragraph.join(" ").as_str(),
            width,
            capabilities,
        ));
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

            if current_width > 0 && current_width == display_width(current_prefix) {
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
            lines.extend(render_table_block(&block, width, capabilities));
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
) -> Vec<String> {
    let rows = lines
        .iter()
        .map(|line| parse_table_row(line))
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

    rows.iter()
        .enumerate()
        .map(|(row_index, row)| {
            if separator_rows[row_index] {
                render_table_separator(&col_widths)
            } else {
                render_table_row(row, &col_widths)
            }
        })
        .collect()
}

fn parse_table_row(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

fn is_table_separator(cell: &str) -> bool {
    let trimmed = cell.trim();
    !trimmed.is_empty()
        && trimmed
            .chars()
            .all(|ch| ch == '-' || ch == ':' || ch.is_whitespace())
}

fn render_table_separator(col_widths: &[usize]) -> String {
    let mut line = String::from("|");
    for width in col_widths {
        line.push_str(&"-".repeat(width.saturating_add(2)));
        line.push('|');
    }
    line
}

fn render_table_row(row: &[String], col_widths: &[usize]) -> String {
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
    use super::wrap_text;
    use crate::capability::{ColorLevel, GlyphMode, TerminalCapabilities};

    fn unicode_capabilities() -> TerminalCapabilities {
        TerminalCapabilities {
            color: ColorLevel::TrueColor,
            glyphs: GlyphMode::Unicode,
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
    fn wrap_text_formats_markdown_tables() {
        let lines = wrap_text(
            "| 工具 | 说明 |\n| --- | --- |\n| reviewnow | 代码审查 |\n| git-commit | 自动提交 |",
            32,
            unicode_capabilities(),
        );
        assert!(lines.iter().any(|line| line.contains("| 工具")));
        assert!(lines.iter().any(|line| line.contains("---")));
    }
}
