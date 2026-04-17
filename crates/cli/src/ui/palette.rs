use super::{ThemePalette, truncate_to_width};
use crate::state::{PaletteState, WrappedLine, WrappedLineStyle};

const MAX_VISIBLE_ITEMS: usize = 5;

pub fn palette_lines(
    palette: &PaletteState,
    width: usize,
    theme: &dyn ThemePalette,
) -> Vec<WrappedLine> {
    match palette {
        PaletteState::Closed => Vec::new(),
        PaletteState::Slash(slash) => render_palette_items(
            &slash.items,
            slash.selected,
            width,
            theme,
            "  没有匹配的命令",
            |item| (item.title.as_str(), item.description.as_str()),
        ),
        PaletteState::Resume(resume) => render_palette_items(
            &resume.items,
            resume.selected,
            width,
            theme,
            "  没有匹配的会话",
            |item| (item.title.as_str(), item.working_dir.as_str()),
        ),
    }
}

pub fn palette_visible(palette: &PaletteState) -> bool {
    !matches!(palette, PaletteState::Closed)
}

fn visible_window<T>(items: &[T], selected: usize, max_items: usize) -> Vec<(usize, &T)> {
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
        return truncate_to_width(format!("{prefix} {title}").as_str(), available);
    }

    let available_meta = available.saturating_mul(3) / 5;
    let meta_text = truncate_to_width(meta.trim(), available_meta.max(8));
    let title_budget = available
        .saturating_sub(unicode_width::UnicodeWidthStr::width(meta_text.as_str()))
        .saturating_sub(3)
        .max(10);
    let title_text = truncate_to_width(title.trim(), title_budget);
    truncate_to_width(
        format!("{prefix} {title_text} — {meta_text}").as_str(),
        available,
    )
}

fn render_palette_items<T, F>(
    items: &[T],
    selected: usize,
    width: usize,
    theme: &dyn ThemePalette,
    empty_message: &str,
    meta: F,
) -> Vec<WrappedLine>
where
    F: Fn(&T) -> (&str, &str),
{
    if items.is_empty() {
        return vec![WrappedLine {
            style: WrappedLineStyle::Muted,
            content: empty_message.to_string(),
        }];
    }

    visible_window(items, selected, MAX_VISIBLE_ITEMS)
        .into_iter()
        .map(|(absolute_index, item)| {
            let (title, details) = meta(item);
            WrappedLine {
                style: if absolute_index == selected {
                    WrappedLineStyle::PaletteSelected
                } else {
                    WrappedLineStyle::PaletteItem
                },
                content: candidate_line(
                    if absolute_index == selected {
                        theme.glyph("›", ">")
                    } else {
                        " "
                    },
                    title,
                    details,
                    width,
                ),
            }
        })
        .collect()
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
