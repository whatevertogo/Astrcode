use unicode_width::UnicodeWidthStr;

use super::{ThemePalette, cells::wrap_text, truncate_to_width};
use crate::{
    capability::TerminalCapabilities,
    state::{CliState, WrappedLine, WrappedLineStyle},
};

const MIN_CARD_WIDTH: usize = 44;
const HORIZONTAL_LAYOUT_WIDTH: usize = 70;

pub fn hero_lines(state: &CliState, width: u16, theme: &dyn ThemePalette) -> Vec<WrappedLine> {
    let width = usize::from(width.max(MIN_CARD_WIDTH as u16));
    let session_title = state
        .conversation
        .active_session_title
        .as_deref()
        .filter(|title| !title.trim().is_empty())
        .unwrap_or("Astrcode workspace");
    let working_dir = state
        .shell
        .working_dir
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "未附加工作目录".to_string());
    let phase = state
        .active_phase()
        .map(|phase| format!("{phase:?}").to_lowercase())
        .unwrap_or_else(|| "idle".to_string());
    let recent_sessions = state
        .conversation
        .sessions
        .iter()
        .filter(|session| {
            Some(session.session_id.as_str()) != state.conversation.active_session_id.as_deref()
        })
        .take(3)
        .map(|session| {
            if session.title.trim().is_empty() {
                session.display_name.clone()
            } else {
                session.title.clone()
            }
        })
        .collect::<Vec<_>>();

    let title = format!(" Astrcode v{} ", env!("CARGO_PKG_VERSION"));
    let context = HeroContext {
        width,
        title: title.as_str(),
        session_title,
        working_dir: working_dir.as_str(),
        phase: phase.as_str(),
        recent_sessions: &recent_sessions,
        capabilities: state.shell.capabilities,
        theme,
    };
    let mut lines = if width >= HORIZONTAL_LAYOUT_WIDTH {
        horizontal_card(&context)
    } else {
        compact_card(&context)
    };
    lines.push(WrappedLine {
        style: WrappedLineStyle::Plain,
        content: String::new(),
    });
    lines
}

struct HeroContext<'a> {
    width: usize,
    title: &'a str,
    session_title: &'a str,
    working_dir: &'a str,
    phase: &'a str,
    recent_sessions: &'a [String],
    capabilities: TerminalCapabilities,
    theme: &'a dyn ThemePalette,
}

fn horizontal_card(context: &HeroContext<'_>) -> Vec<WrappedLine> {
    let inner_width = context.width.saturating_sub(2).max(MIN_CARD_WIDTH - 2);
    let left_width = (inner_width / 2).clamp(24, 38);
    let right_width = inner_width.saturating_sub(left_width + 1);

    let mut rows = vec![
        two_col_row(
            "Welcome back!",
            "使用提示",
            left_width,
            right_width,
            WrappedLineStyle::HeroTitle,
        ),
        two_col_row(
            context.session_title,
            "输入 / 打开 commands",
            left_width,
            right_width,
            WrappedLineStyle::HeroBody,
        ),
        two_col_row(
            "      /\\_/\\\\",
            "Tab 在 transcript / composer 间切换",
            left_width,
            right_width,
            WrappedLineStyle::HeroBody,
        ),
        two_col_row(
            "   .-\"     \"-.",
            "Ctrl+O 展开或收起 thinking",
            left_width,
            right_width,
            WrappedLineStyle::HeroBody,
        ),
        two_col_row(
            format!("phase · {}", context.phase).as_str(),
            "最近活动",
            left_width,
            right_width,
            WrappedLineStyle::HeroFeedTitle,
        ),
    ];

    let cwd_lines = wrap_text(context.working_dir, left_width, context.capabilities);
    let activity_lines = if context.recent_sessions.is_empty() {
        vec!["暂无最近会话".to_string(), "/resume 查看更多".to_string()]
    } else {
        let mut items = context.recent_sessions.to_vec();
        items.push("/resume 查看更多".to_string());
        items
    };
    let line_count = cwd_lines.len().max(activity_lines.len());
    for index in 0..line_count {
        rows.push(two_col_row(
            cwd_lines.get(index).map(String::as_str).unwrap_or(""),
            activity_lines.get(index).map(String::as_str).unwrap_or(""),
            left_width,
            right_width,
            WrappedLineStyle::HeroMuted,
        ));
    }

    framed_rows(rows, context.width, context.title, context.theme)
}

fn compact_card(context: &HeroContext<'_>) -> Vec<WrappedLine> {
    let inner_width = context.width.saturating_sub(2).max(MIN_CARD_WIDTH - 2);
    let mut rows = vec![
        WrappedLine {
            style: WrappedLineStyle::HeroTitle,
            content: pad_to_width("Welcome back!", inner_width),
        },
        WrappedLine {
            style: WrappedLineStyle::HeroBody,
            content: pad_to_width(context.session_title, inner_width),
        },
        WrappedLine {
            style: WrappedLineStyle::HeroBody,
            content: pad_to_width(format!("phase · {}", context.phase).as_str(), inner_width),
        },
    ];

    for line in wrap_text(context.working_dir, inner_width, context.capabilities) {
        rows.push(WrappedLine {
            style: WrappedLineStyle::HeroMuted,
            content: pad_to_width(line.as_str(), inner_width),
        });
    }

    rows.push(WrappedLine {
        style: WrappedLineStyle::HeroFeedTitle,
        content: pad_to_width("使用提示", inner_width),
    });
    for tip in [
        "输入 / 打开 commands",
        "Ctrl+O 展开或收起 thinking",
        "Tab 在 transcript / composer 间切换",
    ] {
        rows.push(WrappedLine {
            style: WrappedLineStyle::HeroBody,
            content: pad_to_width(tip, inner_width),
        });
    }
    rows.push(WrappedLine {
        style: WrappedLineStyle::HeroFeedTitle,
        content: pad_to_width("最近活动", inner_width),
    });
    if context.recent_sessions.is_empty() {
        rows.push(WrappedLine {
            style: WrappedLineStyle::HeroMuted,
            content: pad_to_width("暂无最近会话", inner_width),
        });
    } else {
        for item in context.recent_sessions.iter().take(3) {
            rows.push(WrappedLine {
                style: WrappedLineStyle::HeroMuted,
                content: pad_to_width(item.as_str(), inner_width),
            });
        }
    }
    rows.push(WrappedLine {
        style: WrappedLineStyle::HeroMuted,
        content: pad_to_width("/resume 查看更多", inner_width),
    });

    framed_rows(rows, context.width, context.title, context.theme)
}

fn framed_rows(
    rows: Vec<WrappedLine>,
    width: usize,
    title: &str,
    theme: &dyn ThemePalette,
) -> Vec<WrappedLine> {
    let mut lines = vec![WrappedLine {
        style: WrappedLineStyle::HeroBorder,
        content: frame_top(width, title, theme),
    }];
    let vertical = theme.glyph("│", "|");
    for row in rows {
        lines.push(WrappedLine {
            style: row.style,
            content: format!("{vertical}{}{vertical}", row.content),
        });
    }
    lines.push(WrappedLine {
        style: WrappedLineStyle::HeroBorder,
        content: frame_bottom(width, theme),
    });
    lines
}

fn two_col_row(
    left: &str,
    right: &str,
    left_width: usize,
    right_width: usize,
    style: WrappedLineStyle,
) -> WrappedLine {
    WrappedLine {
        style,
        content: format!(
            "{}│{}",
            pad_to_width(left, left_width),
            pad_to_width(right, right_width)
        ),
    }
}

fn frame_top(width: usize, title: &str, theme: &dyn ThemePalette) -> String {
    let left = theme.glyph("╭", "+");
    let right = theme.glyph("╮", "+");
    let horizontal = theme.glyph("─", "-");
    let inner_width = width.saturating_sub(2);
    let title_width = UnicodeWidthStr::width(title);
    if title_width >= inner_width {
        return format!("{left}{}{right}", truncate_to_width(title, inner_width));
    }
    let remaining = inner_width.saturating_sub(title_width);
    format!("{left}{title}{}{right}", horizontal.repeat(remaining))
}

fn frame_bottom(width: usize, theme: &dyn ThemePalette) -> String {
    let left = theme.glyph("╰", "+");
    let right = theme.glyph("╯", "+");
    let horizontal = theme.glyph("─", "-");
    format!(
        "{left}{}{right}",
        horizontal.repeat(width.saturating_sub(2))
    )
}

fn pad_to_width(text: &str, width: usize) -> String {
    let value = truncate_to_width(text, width);
    let current_width = UnicodeWidthStr::width(value.as_str());
    if current_width >= width {
        return value;
    }
    format!("{value}{}", " ".repeat(width - current_width))
}
