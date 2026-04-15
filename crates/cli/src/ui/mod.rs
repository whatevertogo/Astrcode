use ratatui::text::Line;

use crate::state::{CliState, OverlayState, ResumeOverlayState, SlashPaletteState};

pub fn transcript_lines(state: &CliState) -> Vec<Line<'static>> {
    if state.transcript.is_empty() {
        return vec![Line::from("暂无 transcript，提交 prompt 后会在这里显示。")];
    }

    state
        .transcript
        .iter()
        .map(|block| match block {
            astrcode_client::AstrcodeTerminalBlockDto::User(block) => {
                Line::from(format!("你: {}", block.markdown))
            },
            astrcode_client::AstrcodeTerminalBlockDto::Assistant(block) => Line::from(format!(
                "助手 [{}]: {}",
                status_label(block.status),
                block.markdown
            )),
            astrcode_client::AstrcodeTerminalBlockDto::Thinking(block) => Line::from(format!(
                "Thinking [{}]: {}",
                status_label(block.status),
                block.markdown
            )),
            astrcode_client::AstrcodeTerminalBlockDto::ToolCall(block) => Line::from(format!(
                "Tool {} [{}]{}",
                block.tool_name,
                status_label(block.status),
                block
                    .summary
                    .as_deref()
                    .map(|summary| format!(": {summary}"))
                    .unwrap_or_default()
            )),
            astrcode_client::AstrcodeTerminalBlockDto::ToolStream(block) => Line::from(format!(
                "Tool {:?} [{}]: {}",
                block.stream,
                status_label(block.status),
                block.content
            )),
            astrcode_client::AstrcodeTerminalBlockDto::Error(block) => {
                Line::from(format!("错误 {:?}: {}", block.code, block.message))
            },
            astrcode_client::AstrcodeTerminalBlockDto::SystemNote(block) => {
                Line::from(format!("系统 {:?}: {}", block.note_kind, block.markdown))
            },
            astrcode_client::AstrcodeTerminalBlockDto::ChildHandoff(block) => {
                let message = block.message.as_deref().unwrap_or("无摘要");
                Line::from(format!(
                    "子代理 {:?}: {} ({})",
                    block.handoff_kind, block.child.title, message
                ))
            },
        })
        .collect()
}

pub fn status_line(state: &CliState) -> String {
    let session = state
        .active_session_title
        .as_deref()
        .unwrap_or("未选择会话");
    let phase = state.active_phase().map(phase_label).unwrap_or("unknown");
    format!(
        "session: {session} | phase: {phase} | status: {}",
        state.status.message
    )
}

pub fn overlay_title(state: &CliState) -> Option<&'static str> {
    match state.overlay {
        OverlayState::None => None,
        OverlayState::Resume(_) => Some("恢复会话"),
        OverlayState::SlashPalette(_) => Some("Slash 候选"),
    }
}

pub fn overlay_lines(state: &CliState) -> Vec<Line<'static>> {
    match &state.overlay {
        OverlayState::Resume(resume) => resume_lines(resume),
        OverlayState::SlashPalette(palette) => slash_lines(palette),
        OverlayState::None => Vec::new(),
    }
}

fn resume_lines(resume: &ResumeOverlayState) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(format!("query: {}", resume.query))];
    if resume.items.is_empty() {
        lines.push(Line::from("没有匹配的会话。"));
        return lines;
    }

    lines.extend(resume.items.iter().enumerate().map(|(index, item)| {
        let marker = if index == resume.selected { ">" } else { " " };
        Line::from(format!("{marker} {} | {}", item.title, item.working_dir))
    }));
    lines
}

fn slash_lines(palette: &SlashPaletteState) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(format!("query: {}", palette.query))];
    if palette.items.is_empty() {
        lines.push(Line::from("没有匹配的 slash 候选。"));
        return lines;
    }

    lines.extend(palette.items.iter().enumerate().map(|(index, item)| {
        let marker = if index == palette.selected { ">" } else { " " };
        Line::from(format!(
            "{marker} {} | {}",
            item.action_value, item.description
        ))
    }));
    lines
}

fn status_label(status: astrcode_client::AstrcodeTerminalBlockStatusDto) -> &'static str {
    match status {
        astrcode_client::AstrcodeTerminalBlockStatusDto::Streaming => "streaming",
        astrcode_client::AstrcodeTerminalBlockStatusDto::Complete => "complete",
        astrcode_client::AstrcodeTerminalBlockStatusDto::Failed => "failed",
        astrcode_client::AstrcodeTerminalBlockStatusDto::Cancelled => "cancelled",
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
