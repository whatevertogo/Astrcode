use super::ThemePalette;
use crate::state::{CliState, PaletteState, WrappedLine, WrappedLineStyle};

pub fn footer_lines(state: &CliState, width: u16) -> Vec<WrappedLine> {
    let theme = super::CodexTheme::new(state.shell.capabilities);
    let width = usize::from(width.max(24));
    let prompt = if state.interaction.composer.is_empty() {
        "在这里输入，或键入 /".to_string()
    } else {
        visible_input(
            state.interaction.composer.input.as_str(),
            width.saturating_sub(4),
        )
    };

    vec![
        WrappedLine {
            style: if state.interaction.composer.is_empty() {
                WrappedLineStyle::Muted
            } else {
                WrappedLineStyle::FooterInput
            },
            content: format!("{} {}", theme.glyph("›", ">"), prompt),
        },
        WrappedLine {
            style: if state.interaction.status.is_error {
                WrappedLineStyle::ErrorText
            } else {
                WrappedLineStyle::FooterStatus
            },
            content: truncate(footer_status(state).as_str(), width),
        },
    ]
}

fn footer_status(state: &CliState) -> String {
    if state.interaction.status.is_error {
        return state.interaction.status.message.clone();
    }

    match &state.interaction.palette {
        PaletteState::Slash(palette) => palette
            .items
            .get(palette.selected)
            .map(|item| {
                format!(
                    "{} · {} · Enter 执行 · Esc 关闭",
                    item.title, item.description
                )
            })
            .unwrap_or_else(|| "/ commands · 没有匹配项 · Esc 关闭".to_string()),
        PaletteState::Resume(resume) => resume
            .items
            .get(resume.selected)
            .map(|item| {
                format!(
                    "{} · {} · Enter 切换 · Esc 关闭",
                    item.title, item.working_dir
                )
            })
            .unwrap_or_else(|| "/resume · 没有匹配会话 · Esc 关闭".to_string()),
        PaletteState::Closed if state.interaction.composer.line_count() > 1 => format!(
            "{} 行输入 · Shift+Enter 换行 · Ctrl+O thinking",
            state.interaction.composer.line_count()
        ),
        PaletteState::Closed => {
            let phase = state
                .active_phase()
                .map(|phase| format!("{phase:?}").to_lowercase())
                .unwrap_or_else(|| "idle".to_string());
            let message = state.interaction.status.message.as_str();
            if message.is_empty() || message == "ready" {
                format!("{phase} · Enter 发送 · / commands")
            } else {
                format!("{message} · {phase}")
            }
        },
    }
}

fn visible_input(input: &str, width: usize) -> String {
    let line = input.lines().last().unwrap_or_default();
    if line.chars().count() <= width {
        return line.to_string();
    }
    line.chars()
        .rev()
        .take(width.saturating_sub(1))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>()
}

fn truncate(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    if width <= 1 {
        return "…".to_string();
    }
    let mut value = text.chars().take(width - 1).collect::<String>();
    value.push('…');
    value
}
