use ratatui::{
    Frame,
    layout::{Constraint, Direction, Flex, Layout, Rect},
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::{state::CliState, ui};

pub fn render(frame: &mut Frame<'_>, state: &CliState) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let header = Paragraph::new(Line::from(format!(
        "Astrcode Terminal | {}",
        state.connection_origin
    )));
    frame.render_widget(header, layout[0]);

    let transcript = Paragraph::new(ui::transcript_lines(state))
        .block(Block::default().title("Transcript").borders(Borders::ALL))
        .wrap(Wrap { trim: false })
        .scroll((state.scroll_anchor, 0));
    frame.render_widget(transcript, layout[1]);

    let composer_title = match state.pane_focus {
        crate::state::PaneFocus::Composer => "Composer *",
        _ => "Composer",
    };
    let composer = Paragraph::new(state.composer.input.as_str())
        .block(Block::default().title(composer_title).borders(Borders::ALL));
    frame.render_widget(composer, layout[2]);

    if let Some(banner) = &state.banner {
        let banner_area = centered_rect(70, 5, frame.area());
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
        let overlay = Paragraph::new(ui::overlay_lines(state))
            .block(Block::default().title(title).borders(Borders::ALL))
            .wrap(Wrap { trim: false });
        frame.render_widget(Clear, overlay_area);
        frame.render_widget(overlay, overlay_area);
    }

    let status_style = if state.status.is_error {
        Style::default().fg(Color::Red)
    } else {
        Style::default()
    };
    let status_area = Rect {
        x: layout[2].x,
        y: layout[2].y.saturating_sub(1),
        width: layout[2].width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(ui::status_line(state)).style(status_style),
        status_area,
    );
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
