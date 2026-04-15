use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    capability::{ColorLevel, TerminalCapabilities},
    state::{
        CliState, OverlayState, PaneFocus, ResumeOverlayState, SlashPaletteState, WrappedLine,
        WrappedLineStyle,
    },
};

pub fn transcript_lines(state: &CliState, width: u16) -> Vec<WrappedLine> {
    if state.conversation.transcript.is_empty() {
        return vec![
            WrappedLine {
                style: WrappedLineStyle::Plain,
                content: "Astrcode 已准备好。".to_string(),
            },
            WrappedLine {
                style: WrappedLineStyle::Muted,
                content: "输入 prompt 后，对话会从这里开始。".to_string(),
            },
            WrappedLine {
                style: WrappedLineStyle::Muted,
                content: "输入 / 可查看可用 skill 与命令。".to_string(),
            },
        ];
    }

    let available_width = width.max(8) as usize;
    let mut lines = Vec::new();
    for block in &state.conversation.transcript {
        let (style, title, body) = match block {
            astrcode_client::AstrcodeConversationBlockDto::User(block) => (
                WrappedLineStyle::Accent,
                label(state.shell.capabilities, "你", "you"),
                block.markdown.as_str(),
            ),
            astrcode_client::AstrcodeConversationBlockDto::Assistant(block) => (
                WrappedLineStyle::Plain,
                format!(
                    "{}{}",
                    label(state.shell.capabilities, "Astrcode", "astrcode"),
                    status_suffix(block.status)
                ),
                block.markdown.as_str(),
            ),
            astrcode_client::AstrcodeConversationBlockDto::Thinking(block) => (
                WrappedLineStyle::Muted,
                format!(
                    "{}{}",
                    label(state.shell.capabilities, "思考", "think"),
                    status_suffix(block.status)
                ),
                block.markdown.as_str(),
            ),
            astrcode_client::AstrcodeConversationBlockDto::ToolCall(block) => (
                WrappedLineStyle::Warning,
                format!(
                    "{}/{}{}",
                    label(state.shell.capabilities, "工具", "tool"),
                    block.tool_name,
                    status_suffix(block.status)
                ),
                block.summary.as_deref().unwrap_or("正在执行工具调用"),
            ),
            astrcode_client::AstrcodeConversationBlockDto::ToolStream(block) => (
                WrappedLineStyle::Warning,
                format!(
                    "{}/{:?}{}",
                    label(state.shell.capabilities, "流", "stream"),
                    block.stream,
                    status_suffix(block.status)
                ),
                block.content.as_str(),
            ),
            astrcode_client::AstrcodeConversationBlockDto::Error(block) => (
                WrappedLineStyle::Error,
                format!(
                    "{} {:?}",
                    label(state.shell.capabilities, "错误", "error"),
                    block.code
                ),
                block.message.as_str(),
            ),
            astrcode_client::AstrcodeConversationBlockDto::SystemNote(block) => (
                WrappedLineStyle::Muted,
                format!(
                    "{} {:?}",
                    label(state.shell.capabilities, "系统", "note"),
                    block.note_kind
                ),
                block.markdown.as_str(),
            ),
            astrcode_client::AstrcodeConversationBlockDto::ChildHandoff(block) => (
                WrappedLineStyle::Accent,
                format!(
                    "{}/{} {:?}",
                    label(state.shell.capabilities, "子代理", "child"),
                    block.child.title,
                    block.handoff_kind
                ),
                block.message.as_deref().unwrap_or("无摘要"),
            ),
        };
        lines.extend(wrap_labeled_block(
            &title,
            body,
            style,
            available_width,
            state.shell.capabilities,
        ));
    }
    lines
}

pub fn child_pane_lines(state: &CliState) -> Vec<WrappedLine> {
    if state.conversation.child_summaries.is_empty() {
        return vec![WrappedLine {
            style: WrappedLineStyle::Muted,
            content: "暂无 child agent。".to_string(),
        }];
    }

    let mut lines = Vec::new();
    for (index, child) in state.conversation.child_summaries.iter().enumerate() {
        let selected = index == state.interaction.child_pane.selected;
        let focused = state
            .interaction
            .child_pane
            .focused_child_session_id
            .as_deref()
            == Some(child.child_session_id.as_str());
        let marker = if focused {
            label(state.shell.capabilities, "◆", "*")
        } else if selected {
            label(state.shell.capabilities, "›", ">")
        } else {
            " ".to_string()
        };
        lines.push(WrappedLine {
            style: if focused {
                WrappedLineStyle::Accent
            } else {
                WrappedLineStyle::Plain
            },
            content: format!(
                "{marker} {} [{}]",
                child.title,
                lifecycle_label(child.lifecycle)
            ),
        });
        if let Some(summary) = child.latest_output_summary.as_deref() {
            lines.push(WrappedLine {
                style: WrappedLineStyle::Muted,
                content: format!("  {summary}"),
            });
        }
    }

    if let Some(focused) = state.focused_child_summary() {
        lines.push(WrappedLine {
            style: WrappedLineStyle::Muted,
            content: String::new(),
        });
        lines.push(WrappedLine {
            style: WrappedLineStyle::Accent,
            content: format!(
                "{} {}",
                label(state.shell.capabilities, "聚焦", "FOCUS"),
                focused.title
            ),
        });
        lines.push(WrappedLine {
            style: WrappedLineStyle::Muted,
            content: format!("session: {}", focused.child_session_id),
        });
        lines.push(WrappedLine {
            style: WrappedLineStyle::Muted,
            content: format!("agent: {}", focused.child_agent_id),
        });
    }

    lines
}

pub fn status_line(state: &CliState) -> String {
    let session = state
        .conversation
        .active_session_title
        .as_deref()
        .unwrap_or("未选择会话");
    let phase = state.active_phase().map(phase_label).unwrap_or("unknown");
    let focus = match state.interaction.pane_focus {
        PaneFocus::Transcript => "transcript",
        PaneFocus::ChildPane => "children",
        PaneFocus::Composer => "composer",
        PaneFocus::Overlay => "overlay",
    };
    let mut parts = vec![
        format!("session {session}"),
        format!("phase {phase}"),
        format!("focus {focus}"),
    ];
    if state.stream_view.pending_chunks > 0 {
        parts.push(format!(
            "stream {:?}/{:?}",
            state.stream_view.mode, state.stream_view.pending_chunks
        ));
    }
    if !state.interaction.status.message.is_empty() {
        parts.push(state.interaction.status.message.clone());
    }

    if matches!(state.shell.capabilities.color, ColorLevel::None) {
        parts.push("mono".to_string());
    };

    parts.join(" · ")
}

pub fn overlay_lines(state: &CliState) -> Vec<WrappedLine> {
    match &state.interaction.overlay {
        OverlayState::Resume(resume) => resume_lines(resume),
        OverlayState::SlashPalette(palette) => slash_lines(palette),
        OverlayState::None => Vec::new(),
    }
}

pub fn overlay_title(state: &CliState) -> Option<&'static str> {
    match state.interaction.overlay {
        OverlayState::None => None,
        OverlayState::Resume(_) => Some("恢复会话"),
        OverlayState::SlashPalette(_) => Some("Slash 候选"),
    }
}

pub fn line_to_ratatui(line: &WrappedLine, capabilities: TerminalCapabilities) -> Line<'static> {
    Line::from(Span::styled(
        line.content.clone(),
        line_style(line.style, capabilities),
    ))
}

fn line_style(style: WrappedLineStyle, capabilities: TerminalCapabilities) -> Style {
    let base = Style::default();
    if matches!(capabilities.color, ColorLevel::None) {
        return if matches!(style, WrappedLineStyle::Accent | WrappedLineStyle::Warning) {
            base.add_modifier(Modifier::BOLD)
        } else {
            base
        };
    }

    match style {
        WrappedLineStyle::Plain => base,
        WrappedLineStyle::Muted => base.fg(Color::DarkGray),
        WrappedLineStyle::Accent => base.fg(Color::Cyan).add_modifier(Modifier::BOLD),
        WrappedLineStyle::Warning => base.fg(Color::Yellow),
        WrappedLineStyle::Error => base.fg(Color::Red).add_modifier(Modifier::BOLD),
    }
}

fn resume_lines(resume: &ResumeOverlayState) -> Vec<WrappedLine> {
    let mut lines = vec![WrappedLine {
        style: WrappedLineStyle::Muted,
        content: format!("query: {}", resume.query),
    }];
    if resume.items.is_empty() {
        lines.push(WrappedLine {
            style: WrappedLineStyle::Muted,
            content: "没有匹配的会话。".to_string(),
        });
        return lines;
    }

    lines.extend(
        resume
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| WrappedLine {
                style: if index == resume.selected {
                    WrappedLineStyle::Accent
                } else {
                    WrappedLineStyle::Plain
                },
                content: format!("{} | {}", item.title, item.working_dir),
            }),
    );
    lines
}

fn slash_lines(palette: &SlashPaletteState) -> Vec<WrappedLine> {
    let mut lines = vec![WrappedLine {
        style: WrappedLineStyle::Muted,
        content: format!("query: {}", palette.query),
    }];
    if palette.items.is_empty() {
        lines.push(WrappedLine {
            style: WrappedLineStyle::Muted,
            content: "没有匹配的 slash 候选。".to_string(),
        });
        return lines;
    }

    lines.extend(
        palette
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| WrappedLine {
                style: if index == palette.selected {
                    WrappedLineStyle::Accent
                } else {
                    WrappedLineStyle::Plain
                },
                content: format!("{} | {}", item.action_value, item.description),
            }),
    );
    lines
}

fn wrap_labeled_block(
    title: &str,
    body: &str,
    style: WrappedLineStyle,
    width: usize,
    capabilities: TerminalCapabilities,
) -> Vec<WrappedLine> {
    let prefix = format!("{title}: ");
    let prefix_width = display_width(prefix.as_str(), capabilities);
    let body_width = width.saturating_sub(prefix_width).max(8);
    let wrapped = wrap_text(body, body_width, capabilities);
    wrapped
        .into_iter()
        .enumerate()
        .map(|(index, content)| WrappedLine {
            style,
            content: if index == 0 {
                format!("{prefix}{content}")
            } else {
                format!("{}{}", " ".repeat(prefix_width), content)
            },
        })
        .collect()
}

fn wrap_text(text: &str, width: usize, capabilities: TerminalCapabilities) -> Vec<String> {
    let normalized = if text.trim().is_empty() {
        " ".to_string()
    } else {
        text.to_string()
    };
    let mut lines = Vec::new();
    for raw_line in normalized.lines() {
        let mut current = String::new();
        for word in raw_line.split_whitespace() {
            let candidate = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            if display_width(candidate.as_str(), capabilities) > width && !current.is_empty() {
                lines.push(current);
                current = word.to_string();
            } else {
                current = candidate;
            }
        }
        if current.is_empty() {
            lines.push(String::new());
        } else {
            lines.push(current);
        }
    }
    lines
}

fn display_width(text: &str, capabilities: TerminalCapabilities) -> usize {
    if capabilities.ascii_only() {
        text.chars().count()
    } else {
        UnicodeWidthStr::width(text)
    }
}

fn label(capabilities: TerminalCapabilities, unicode: &str, ascii: &str) -> String {
    if capabilities.ascii_only() {
        ascii.to_string()
    } else {
        unicode.to_string()
    }
}

fn status_suffix(status: astrcode_client::AstrcodeConversationBlockStatusDto) -> &'static str {
    match status {
        astrcode_client::AstrcodeConversationBlockStatusDto::Streaming => " · streaming",
        astrcode_client::AstrcodeConversationBlockStatusDto::Complete => "",
        astrcode_client::AstrcodeConversationBlockStatusDto::Failed => " · failed",
        astrcode_client::AstrcodeConversationBlockStatusDto::Cancelled => " · cancelled",
    }
}

fn phase_label(phase: astrcode_client::AstrcodePhaseDto) -> &'static str {
    match phase {
        astrcode_client::AstrcodePhaseDto::Idle => "idle",
        astrcode_client::AstrcodePhaseDto::Thinking => "thinking",
        astrcode_client::AstrcodePhaseDto::CallingTool => "calling_tool",
        astrcode_client::AstrcodePhaseDto::Streaming => "streaming",
        astrcode_client::AstrcodePhaseDto::Interrupted => "interrupted",
        astrcode_client::AstrcodePhaseDto::Done => "done",
    }
}

fn lifecycle_label(
    lifecycle: astrcode_client::AstrcodeConversationAgentLifecycleDto,
) -> &'static str {
    match lifecycle {
        astrcode_client::AstrcodeConversationAgentLifecycleDto::Pending => "pending",
        astrcode_client::AstrcodeConversationAgentLifecycleDto::Running => "running",
        astrcode_client::AstrcodeConversationAgentLifecycleDto::Idle => "idle",
        astrcode_client::AstrcodeConversationAgentLifecycleDto::Terminated => "terminated",
    }
}
