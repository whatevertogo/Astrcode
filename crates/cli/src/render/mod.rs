use ratatui::{
    Frame,
    layout::{Constraint, Direction, Flex, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
};

use crate::{
    capability::ColorLevel,
    state::{CliState, PaneFocus, WrappedLineStyle},
    ui,
};

const BRAND_WORDMARK: &str = " Astrcode ";

pub fn render(frame: &mut Frame<'_>, state: &mut CliState) {
    state.set_viewport_size(frame.area().width, frame.area().height);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(frame.area());

    frame.render_widget(brand_rule(state, layout[0]), layout[0]);

    let body = if state.conversation.child_summaries.is_empty() {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(1)])
            .split(layout[1])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(72),
                Constraint::Length(2),
                Constraint::Percentage(28),
            ])
            .split(layout[1])
    };

    let transcript_width = body[0].width;
    let transcript_lines = if state.render.transcript_cache.width == transcript_width
        && state.render.transcript_cache.revision == state.render.transcript_revision
    {
        state.render.transcript_cache.lines.clone()
    } else {
        let lines = ui::transcript_lines(state, transcript_width);
        state.update_transcript_cache(transcript_width, lines.clone());
        lines
    };
    let transcript = Paragraph::new(
        transcript_lines
            .iter()
            .map(|line| ui::line_to_ratatui(line, state.shell.capabilities))
            .collect::<Vec<_>>(),
    )
    .wrap(Wrap { trim: false })
    .scroll((state.interaction.scroll_anchor, 0));
    frame.render_widget(transcript, body[0]);

    if body.len() > 1 {
        frame.render_widget(
            Paragraph::new(vertical_divider(state, body[1].height)).style(separator_style(state)),
            body[1],
        );
        let child_items = ui::child_pane_lines(state)
            .into_iter()
            .map(|line| {
                ListItem::new(ui::line_to_ratatui(&line, state.shell.capabilities)).style(
                    match line.style {
                        WrappedLineStyle::Error => Style::default().fg(Color::Red),
                        _ => Style::default(),
                    },
                )
            })
            .collect::<Vec<_>>();
        let child_pane = List::new(child_items);
        frame.render_widget(child_pane, body[2]);
    }

    let status_style = if state.interaction.status.is_error {
        Style::default().fg(Color::Red)
    } else if state.interaction.pane_focus == PaneFocus::Overlay {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    frame.render_widget(
        Paragraph::new(ui::status_line(state)).style(status_style),
        layout[2],
    );
    frame.render_widget(composer_line(state), layout[3]);
    frame.render_widget(bottom_rule(state), layout[4]);

    if let Some(banner) = &state.conversation.banner {
        let banner_area = centered_rect(72, 5, frame.area());
        let banner_widget = Paragraph::new(banner.error.message.as_str())
            .block(
                Block::default()
                    .title("Connection")
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Red)),
            )
            .wrap(Wrap { trim: false });
        frame.render_widget(Clear, banner_area);
        frame.render_widget(banner_widget, banner_area);
    }

    if let Some(title) = ui::overlay_title(state) {
        let overlay_area = centered_rect(75, 12, frame.area());
        let overlay = Paragraph::new(
            ui::overlay_lines(state)
                .iter()
                .map(|line| ui::line_to_ratatui(line, state.shell.capabilities))
                .collect::<Vec<_>>(),
        )
        .block(Block::default().title(title).borders(Borders::ALL))
        .wrap(Wrap { trim: false });
        frame.render_widget(Clear, overlay_area);
        frame.render_widget(overlay, overlay_area);
    }
}

fn centered_rect(width_percent: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(area.height.saturating_sub(height) / 2),
            Constraint::Length(height),
            Constraint::Min(0),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(width_percent)])
        .flex(Flex::Center)
        .split(vertical[1])[0]
}

fn brand_rule(state: &CliState, area: Rect) -> Paragraph<'static> {
    let glyph = rule_glyph(state);
    let label = BRAND_WORDMARK;
    let width = area.width as usize;
    let label_width = label.chars().count();
    let left_width = width.saturating_sub(label_width) / 2;
    let right_width = width.saturating_sub(label_width + left_width);
    let line = format!(
        "{}{}{}",
        glyph.repeat(left_width),
        label,
        glyph.repeat(right_width)
    );
    Paragraph::new(line).style(Style::default().add_modifier(Modifier::DIM))
}

fn composer_line(state: &CliState) -> Paragraph<'static> {
    let prompt = if state.interaction.composer.input.is_empty() {
        if state.interaction.pane_focus == PaneFocus::Composer {
            "› 开始聊天，Enter 提交，/ 查看命令".to_string()
        } else {
            "  开始聊天，Tab 可切换 focus".to_string()
        }
    } else if state.interaction.pane_focus == PaneFocus::Composer {
        format!("› {}", state.interaction.composer.input)
    } else {
        format!("  {}", state.interaction.composer.input)
    };
    Paragraph::new(prompt).style(if state.interaction.pane_focus == PaneFocus::Composer {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default()
    })
}

fn bottom_rule(state: &CliState) -> Paragraph<'static> {
    Paragraph::new(rule_glyph(state).repeat(usize::from(state.render.viewport_width.max(1))))
        .style(separator_style(state))
}

fn vertical_divider(state: &CliState, height: u16) -> String {
    let glyph = if state.shell.capabilities.ascii_only() {
        "|"
    } else {
        "│"
    };
    std::iter::repeat_n(glyph, height as usize)
        .collect::<Vec<_>>()
        .join("\n")
}

fn rule_glyph(state: &CliState) -> &'static str {
    if state.shell.capabilities.ascii_only() {
        "-"
    } else {
        "─"
    }
}

fn separator_style(state: &CliState) -> Style {
    let style = Style::default().add_modifier(Modifier::DIM);
    match state.shell.capabilities.color {
        ColorLevel::None => style,
        ColorLevel::Ansi16 | ColorLevel::TrueColor => style.fg(Color::DarkGray),
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::render;
    use crate::{
        capability::{ColorLevel, GlyphMode, TerminalCapabilities},
        state::{CliState, PaneFocus},
    };

    fn capabilities(glyphs: GlyphMode) -> TerminalCapabilities {
        TerminalCapabilities {
            color: ColorLevel::Ansi16,
            glyphs,
            alt_screen: false,
            mouse: false,
            bracketed_paste: false,
        }
    }

    #[test]
    fn renders_empty_state_and_composer() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = CliState::new(
            "http://127.0.0.1:5529".to_string(),
            None,
            capabilities(GlyphMode::Unicode),
        );

        terminal
            .draw(|frame| render(frame, &mut state))
            .expect("draw");
        let buffer = terminal.backend().buffer().clone();
        let text = buffer
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(text.contains("Astrcode"));
        assert_eq!(state.render.transcript_cache.lines.len(), 3);
        assert!(
            state.render.transcript_cache.lines[0]
                .content
                .contains("Astrcode 已准备好")
        );
    }

    #[test]
    fn renders_degrade_mode_in_ascii() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = CliState::new(
            "http://127.0.0.1:5529".to_string(),
            None,
            capabilities(GlyphMode::Ascii),
        );
        state.interaction.status.message = "ready".to_string();

        terminal
            .draw(|frame| render(frame, &mut state))
            .expect("draw");
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Astrcode"));
        assert!(text.contains("-"));
    }

    #[test]
    fn renders_child_pane_title_when_children_exist() {
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        let mut state = CliState::new(
            "http://127.0.0.1:5529".to_string(),
            None,
            capabilities(GlyphMode::Unicode),
        );
        state.conversation.child_summaries.push(
            astrcode_client::AstrcodeConversationChildSummaryDto {
                child_session_id: "child-1".to_string(),
                child_agent_id: "agent-1".to_string(),
                title: "Repo inspector".to_string(),
                lifecycle: astrcode_client::AstrcodeConversationAgentLifecycleDto::Running,
                latest_output_summary: Some("checking repo".to_string()),
                child_ref: None,
            },
        );
        state.interaction.pane_focus = PaneFocus::ChildPane;

        terminal
            .draw(|frame| render(frame, &mut state))
            .expect("draw");
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Repo inspector"));
    }
}
