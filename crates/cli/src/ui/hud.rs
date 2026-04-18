use ratatui::layout::Rect;
#[cfg(test)]
use unicode_width::UnicodeWidthStr;

#[cfg(test)]
use crate::ui::truncate_to_width;
use crate::{
    bottom_pane::{BottomPaneState, render_bottom_pane},
    chat::ChatSurfaceState,
    state::CliState,
    ui::{CodexTheme, custom_terminal::Frame, overlay::render_browser_overlay},
};

pub fn render_hud(frame: &mut Frame<'_>, state: &CliState, theme: &CodexTheme) {
    render_hud_in_area(frame, frame.area(), state, theme)
}

pub fn render_hud_in_area(frame: &mut Frame<'_>, area: Rect, state: &CliState, theme: &CodexTheme) {
    if state.interaction.browser.open {
        render_browser_overlay(frame, state, theme);
        return;
    }

    let mut chat = ChatSurfaceState::default();
    let chat_frame = chat.build_frame(state, theme, area.width);
    let pane = BottomPaneState::from_cli(state, &chat_frame, theme, area.width);
    render_bottom_pane(frame, area, state, &pane, theme);
}

pub fn desired_viewport_height(state: &CliState, total_height: u16) -> u16 {
    if state.interaction.browser.open {
        return total_height.max(1);
    }
    let theme = CodexTheme::new(state.shell.capabilities);
    let mut chat = ChatSurfaceState::default();
    let chat_frame = chat.build_frame(state, &theme, 80);
    BottomPaneState::from_cli(state, &chat_frame, &theme, 80).desired_height(total_height)
}

#[cfg(test)]
fn align_left_right(left: &str, right: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let left = truncate_to_width(left, width);
    let right = truncate_to_width(right, width);
    let left_width = UnicodeWidthStr::width(left.as_str());
    let right_width = UnicodeWidthStr::width(right.as_str());
    if left_width + right_width + 1 > width {
        return truncate_to_width(format!("{left} · {right}").as_str(), width);
    }
    format!(
        "{left}{}{right}",
        " ".repeat(width - left_width - right_width)
    )
}

#[cfg(test)]
mod tests {
    use astrcode_client::{
        AstrcodeConversationAssistantBlockDto, AstrcodeConversationBlockDto,
        AstrcodeConversationBlockStatusDto,
    };

    use super::{align_left_right, desired_viewport_height};
    use crate::{
        capability::{ColorLevel, GlyphMode, TerminalCapabilities},
        state::CliState,
    };

    fn capabilities() -> TerminalCapabilities {
        TerminalCapabilities {
            color: ColorLevel::Ansi16,
            glyphs: GlyphMode::Unicode,
            alt_screen: false,
            mouse: false,
            bracketed_paste: false,
        }
    }

    #[test]
    fn desired_viewport_height_stays_small() {
        let mut state = CliState::new("http://127.0.0.1:5529".to_string(), None, capabilities());
        assert!((6..=8).contains(&desired_viewport_height(&state, 20)));

        state.conversation.transcript = vec![AstrcodeConversationBlockDto::Assistant(
            AstrcodeConversationAssistantBlockDto {
                id: "assistant-streaming".to_string(),
                turn_id: Some("turn-1".to_string()),
                status: AstrcodeConversationBlockStatusDto::Streaming,
                markdown: "这是一个比较长的流式响应，用来验证底部面板会扩展。".to_string(),
            },
        )];
        assert!((2..=5).contains(&desired_viewport_height(&state, 20)));
    }

    #[test]
    fn align_left_right_preserves_right_hint() {
        let line = align_left_right("Esc close", "glm-5.1 · idle · Astrcode", 40);
        assert!(line.contains("glm-5.1"));
    }
}
