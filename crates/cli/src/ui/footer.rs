use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{ThemePalette, truncate_to_width};
use crate::state::{
    CliState, ComposerState, PaletteState, PaneFocus, WrappedLine, WrappedLineStyle,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FooterRenderOutput {
    pub lines: Vec<WrappedLine>,
    pub cursor_col: u16,
}

pub fn footer_lines(state: &CliState, width: u16, theme: &dyn ThemePalette) -> FooterRenderOutput {
    let width = usize::from(width.max(24));
    let prompt_state = visible_input_state(&state.interaction.composer, width.saturating_sub(4));
    let input_focused = matches!(
        state.interaction.pane_focus,
        PaneFocus::Composer | PaneFocus::Palette
    );
    let prompt = if state.interaction.composer.is_empty() {
        if input_focused {
            String::new()
        } else {
            "在这里输入，或键入 /".to_string()
        }
    } else {
        prompt_state.visible
    };

    FooterRenderOutput {
        lines: vec![
            WrappedLine {
                style: if state.interaction.status.is_error {
                    WrappedLineStyle::ErrorText
                } else {
                    WrappedLineStyle::FooterStatus
                },
                content: truncate_to_width(status_line(state).as_str(), width),
            },
            WrappedLine {
                style: if state.interaction.composer.is_empty() {
                    if input_focused {
                        WrappedLineStyle::FooterInput
                    } else {
                        WrappedLineStyle::Muted
                    }
                } else {
                    WrappedLineStyle::FooterInput
                },
                content: format!("{} {}", theme.glyph("›", ">"), prompt),
            },
            WrappedLine {
                style: WrappedLineStyle::FooterHint,
                content: truncate_to_width(footer_hint(state).as_str(), width),
            },
        ],
        cursor_col: (2 + prompt_state.cursor_columns.min(width.saturating_sub(3))) as u16,
    }
}

fn status_line(state: &CliState) -> String {
    if state.interaction.status.is_error {
        return state.interaction.status.message.clone();
    }

    match &state.interaction.palette {
        PaletteState::Slash(palette) => format!(
            "/ commands · {} 条候选 · ↑↓ 选择 · Enter 执行 · Esc 关闭",
            palette.items.len()
        ),
        PaletteState::Resume(resume) => format!(
            "/resume · {} 条会话 · ↑↓ 选择 · Enter 切换 · Esc 关闭",
            resume.items.len()
        ),
        PaletteState::Closed => {
            let status = state.interaction.status.message.trim();
            if status.is_empty() || status == "ready" {
                String::new()
            } else {
                status.to_string()
            }
        },
    }
}

fn footer_hint(state: &CliState) -> String {
    if !matches!(state.interaction.palette, PaletteState::Closed) {
        return "Tab 切换焦点 · Esc 关闭 palette · Ctrl+O thinking".to_string();
    }

    let session = state
        .conversation
        .active_session_title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or("新会话");
    let phase = state
        .active_phase()
        .map(|phase| format!("{phase:?}").to_lowercase())
        .unwrap_or_else(|| "idle".to_string());

    if state.interaction.composer.line_count() > 1 {
        format!(
            "{session} · {phase} · {} 行输入 · Shift+Enter 换行 · Ctrl+O thinking",
            state.interaction.composer.line_count()
        )
    } else {
        format!("{session} · {phase} · Enter 发送 · / commands · Ctrl+O thinking")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VisibleInputState {
    visible: String,
    cursor_columns: usize,
}

fn visible_input_state(composer: &ComposerState, width: usize) -> VisibleInputState {
    let input = composer.as_str();
    let cursor = composer.cursor.min(input.len());
    let line_start = input
        .get(..cursor)
        .and_then(|value| value.rfind('\n').map(|index| index + 1))
        .unwrap_or(0);
    let line_end = input
        .get(cursor..)
        .and_then(|value| value.find('\n').map(|index| cursor + index))
        .unwrap_or(input.len());
    let line = &input[line_start..line_end];
    let cursor_in_line = cursor.saturating_sub(line_start);
    let before_cursor = &line[..cursor_in_line];

    if UnicodeWidthStr::width(line) <= width {
        return VisibleInputState {
            visible: line.to_string(),
            cursor_columns: UnicodeWidthStr::width(before_cursor),
        };
    }

    let left_context_budget = width.saturating_mul(2) / 3;
    let mut visible_before = String::new();
    let mut visible_before_width = 0;
    for ch in before_cursor.chars().rev() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if visible_before_width + ch_width > left_context_budget && !visible_before.is_empty() {
            break;
        }
        visible_before.insert(0, ch);
        visible_before_width += ch_width;
    }

    let mut visible = visible_before.clone();
    let mut visible_width = visible_before_width;
    for ch in line[cursor_in_line..].chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if visible_width + ch_width > width {
            break;
        }
        visible.push(ch);
        visible_width += ch_width;
    }

    VisibleInputState {
        cursor_columns: visible_before_width,
        visible,
    }
}
