use super::ThemePalette;
use crate::state::{PaletteState, WrappedLine, WrappedLineStyle};

pub fn palette_lines(
    palette: &PaletteState,
    width: usize,
    theme: &dyn ThemePalette,
) -> Vec<WrappedLine> {
    match palette {
        PaletteState::Closed => Vec::new(),
        PaletteState::Slash(slash) => {
            let mut lines = vec![WrappedLine {
                style: WrappedLineStyle::PaletteTitle,
                content: format!(
                    "{} {}",
                    theme.glyph("/", "/"),
                    if slash.query.is_empty() {
                        "commands".to_string()
                    } else {
                        format!("commands · {}", slash.query)
                    }
                ),
            }];
            if slash.items.is_empty() {
                lines.push(WrappedLine {
                    style: WrappedLineStyle::Muted,
                    content: "  没有匹配的命令".to_string(),
                });
                return lines;
            }
            for (absolute_index, item) in visible_window(&slash.items, slash.selected, 8) {
                lines.push(WrappedLine {
                    style: if absolute_index == slash.selected {
                        WrappedLineStyle::Selection
                    } else {
                        WrappedLineStyle::PaletteItem
                    },
                    content: candidate_line(
                        if absolute_index == slash.selected {
                            theme.glyph("›", ">")
                        } else {
                            " "
                        },
                        item.title.as_str(),
                        item.description.as_str(),
                        width,
                    ),
                });
            }
            lines
        },
        PaletteState::Resume(resume) => {
            let mut lines = vec![WrappedLine {
                style: WrappedLineStyle::PaletteTitle,
                content: format!(
                    "{} {}",
                    theme.glyph("/", "/"),
                    if resume.query.is_empty() {
                        "resume".to_string()
                    } else {
                        format!("resume · {}", resume.query)
                    }
                ),
            }];
            if resume.items.is_empty() {
                lines.push(WrappedLine {
                    style: WrappedLineStyle::Muted,
                    content: "  没有匹配的会话".to_string(),
                });
                return lines;
            }
            for (absolute_index, item) in visible_window(&resume.items, resume.selected, 8) {
                lines.push(WrappedLine {
                    style: if absolute_index == resume.selected {
                        WrappedLineStyle::Selection
                    } else {
                        WrappedLineStyle::PaletteItem
                    },
                    content: candidate_line(
                        if absolute_index == resume.selected {
                            theme.glyph("›", ">")
                        } else {
                            " "
                        },
                        item.title.as_str(),
                        item.working_dir.as_str(),
                        width,
                    ),
                });
            }
            lines
        },
    }
}

pub fn palette_visible(palette: &PaletteState) -> bool {
    !matches!(palette, PaletteState::Closed)
}

fn visible_window<'a, T>(items: &'a [T], selected: usize, max_items: usize) -> Vec<(usize, &'a T)> {
    if items.is_empty() || max_items == 0 {
        return Vec::new();
    }
    let total = items.len();
    let start = if total <= max_items {
        0
    } else {
        selected
            .saturating_sub(max_items / 2)
            .min(total - max_items)
    };
    items[start..(start + max_items).min(total)]
        .iter()
        .enumerate()
        .map(|(offset, item)| (start + offset, item))
        .collect()
}

fn candidate_line(prefix: &str, title: &str, meta: &str, width: usize) -> String {
    let available = width.saturating_sub(2);
    if meta.trim().is_empty() {
        return truncate_with_ellipsis(format!("{prefix} {title}").as_str(), available);
    }

    let meta_text = truncate_with_ellipsis(meta.trim(), available.saturating_mul(3) / 5);
    let title_budget = available
        .saturating_sub(meta_text.chars().count())
        .saturating_sub(3)
        .max(8);
    let title_text = truncate_with_ellipsis(title.trim(), title_budget);
    truncate_with_ellipsis(
        format!("{prefix} {title_text} · {meta_text}").as_str(),
        available,
    )
}

fn truncate_with_ellipsis(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    if width <= 1 {
        return "…".to_string();
    }
    let mut truncated = text.chars().take(width - 1).collect::<String>();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use super::{candidate_line, visible_window};

    #[test]
    fn visible_window_tracks_selected_item() {
        let items = (0..12).collect::<Vec<_>>();
        let window = visible_window(&items, 10, 4);
        let indexes = window
            .into_iter()
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        assert_eq!(indexes, vec![8, 9, 10, 11]);
    }

    #[test]
    fn candidate_line_stays_single_row() {
        let line = candidate_line(
            ">",
            "Issue Fixer",
            "automatically fix GitHub issues and create pull requests",
            48,
        );
        assert!(!line.contains('\n'));
        assert!(line.contains("Issue Fixer"));
    }
}
