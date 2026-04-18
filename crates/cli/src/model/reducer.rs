use std::{collections::BTreeSet, sync::Arc, time::Duration};

use ratatui::text::Line;

use super::events::Event;
use crate::render::wrap::wrap_plain_text;

const STREAM_TAIL_BUDGET: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitKind {
    UserTurn,
    AssistantBlock,
    ToolSummary,
    SystemNote,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedSlice {
    pub kind: CommitKind,
    pub height: u16,
    pub lines: Arc<[Line<'static>]>,
}

impl CommittedSlice {
    pub fn plain(lines: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        let lines = lines
            .into_iter()
            .map(|line| Line::from(line.as_ref().to_string()))
            .collect::<Vec<_>>();
        Self {
            kind: CommitKind::SystemNote,
            height: lines.len().max(1) as u16,
            lines: lines.into(),
        }
    }

    fn new(kind: CommitKind, lines: Vec<Line<'static>>, wrap_width: usize) -> Self {
        let lines = with_block_spacing(lines);
        Self {
            kind,
            height: rendered_height(&lines, wrap_width),
            lines: lines.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HudState {
    pub status_line: Option<Line<'static>>,
    pub detail_lines: Vec<Line<'static>>,
    pub live_preview_lines: Vec<Line<'static>>,
    pub queued_lines: Vec<Line<'static>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OverlayState {
    pub browser_open: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UiProjection {
    pub commit_queue: Vec<CommittedSlice>,
    pub hud: HudState,
    pub overlay: OverlayState,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProjectionReducer {
    committed_once: BTreeSet<String>,
}

impl ProjectionReducer {
    pub fn reset(&mut self) {
        self.committed_once.clear();
    }

    pub fn reduce_events(
        &mut self,
        events: &[Event],
        width: u16,
        hud_height: u16,
        stream_age: Duration,
    ) -> UiProjection {
        let wrap_width = usize::from(width.max(24));
        let content_width = wrap_width.saturating_sub(2);
        let stream_tail_budget = if hud_height <= 10 {
            6
        } else {
            STREAM_TAIL_BUDGET
        };
        let preview_budget = if hud_height <= 10 { 4 } else { 6 };
        let mut projection = UiProjection::default();

        let mut live_details = Vec::new();
        let mut live_preview_lines = Vec::new();
        let mut streaming_assistant_text = None;
        let mut tool_chain_active = false;
        for event in events {
            match event {
                Event::UserTurn { id, text } => {
                    if self.committed_once.insert(id.clone()) {
                        projection.commit_queue.push(CommittedSlice::new(
                            CommitKind::UserTurn,
                            render_prefixed_block("› ", "  ", text, content_width),
                            wrap_width,
                        ));
                    }
                },
                Event::AssistantBlock {
                    id,
                    text,
                    streaming,
                } => {
                    if *streaming {
                        streaming_assistant_text = Some(text.clone());
                        projection.hud.status_line = Some(Line::from("• Responding"));
                    } else if self.committed_once.insert(id.clone()) {
                        projection.commit_queue.push(CommittedSlice::new(
                            CommitKind::AssistantBlock,
                            render_prefixed_block("• ", "  ", text.as_str(), content_width),
                            wrap_width,
                        ));
                    }
                },
                Event::Thinking {
                    summary, preview, ..
                } => {
                    if live_details.is_empty() {
                        live_details = vec![
                            Line::from(format!("  └ {summary}")),
                            Line::from(format!("    {preview}")),
                        ];
                        trim_tail(&mut live_details, stream_tail_budget);
                    }
                    tool_chain_active = true;
                    projection.hud.status_line = Some(Line::from("• Thinking"));
                },
                Event::ToolStatus {
                    tool_name, summary, ..
                } => {
                    live_details = render_detail_block(
                        "└ ",
                        "  ",
                        format!("{tool_name} · {summary}").as_str(),
                        content_width,
                    );
                    trim_tail(&mut live_details, 3);
                    tool_chain_active = true;
                    projection.hud.status_line = Some(Line::from(format!("• Running {tool_name}")));
                },
                Event::ToolSummary {
                    id,
                    tool_name,
                    summary,
                    artifact_path,
                } => {
                    tool_chain_active = true;
                    if self.committed_once.insert(id.clone()) {
                        let mut lines = render_prefixed_block(
                            "↳ ",
                            "  ",
                            format!("{tool_name} · {summary}").as_str(),
                            content_width,
                        );
                        if let Some(path) = artifact_path {
                            lines.push(Line::from(format!("  {path}")));
                        }
                        if lines.len() > 2 {
                            lines = vec![lines[0].clone(), lines[lines.len() - 1].clone()];
                        }
                        projection.commit_queue.push(CommittedSlice::new(
                            CommitKind::ToolSummary,
                            lines,
                            wrap_width,
                        ));
                    }
                },
                Event::SystemNote { id, text } => {
                    if self.committed_once.insert(id.clone()) {
                        projection.commit_queue.push(CommittedSlice::new(
                            CommitKind::SystemNote,
                            render_prefixed_block("· ", "  ", text, content_width),
                            wrap_width,
                        ));
                    }
                },
                Event::Error { id, text } => {
                    if self.committed_once.insert(id.clone()) {
                        projection.commit_queue.push(CommittedSlice::new(
                            CommitKind::Error,
                            render_prefixed_block("! ", "  ", text, content_width),
                            wrap_width,
                        ));
                    }
                    projection.hud.status_line = Some(Line::from(format!("• {text}")));
                },
            }
        }

        if let Some(text) = streaming_assistant_text {
            let preview_triggered = assistant_preview_triggered(
                text.as_str(),
                content_width,
                tool_chain_active,
                stream_age,
            );
            if preview_triggered {
                live_preview_lines = render_preview_block(text.as_str(), content_width);
                trim_tail(&mut live_preview_lines, preview_budget);
            } else if live_details.is_empty() {
                live_details = vec![Line::from("  └ 正在生成回复")];
            }
        }

        projection.hud.detail_lines = live_details;
        projection.hud.live_preview_lines = live_preview_lines;
        projection
    }
}

fn render_prefixed_block(
    first_prefix: &str,
    subsequent_prefix: &str,
    text: &str,
    width: usize,
) -> Vec<Line<'static>> {
    let prefix_width = first_prefix
        .chars()
        .count()
        .max(subsequent_prefix.chars().count());
    let wrapped = wrap_plain_text(text, width.saturating_sub(prefix_width).max(1));
    wrapped
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                Line::from(format!("{first_prefix}{line}"))
            } else {
                Line::from(format!("{subsequent_prefix}{line}"))
            }
        })
        .collect()
}

fn with_block_spacing(mut lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    if !lines
        .last()
        .is_some_and(|line| line.to_string().trim().is_empty())
    {
        lines.push(Line::from(String::new()));
    }
    lines
}

fn trim_tail(lines: &mut Vec<Line<'static>>, limit: usize) {
    if lines.len() > limit {
        let keep_from = lines.len() - limit;
        lines.drain(0..keep_from);
    }
}

fn render_detail_block(
    first_prefix: &str,
    subsequent_prefix: &str,
    text: &str,
    width: usize,
) -> Vec<Line<'static>> {
    let first = format!("  {first_prefix}");
    let subsequent = format!("  {subsequent_prefix}");
    render_prefixed_block(first.as_str(), subsequent.as_str(), text, width)
}

fn render_preview_block(text: &str, width: usize) -> Vec<Line<'static>> {
    wrap_plain_text(text, width.max(1))
        .into_iter()
        .map(Line::from)
        .collect()
}

fn assistant_preview_triggered(
    text: &str,
    width: usize,
    tool_chain_active: bool,
    stream_age: Duration,
) -> bool {
    let rendered_rows = wrap_plain_text(text, width.max(1)).len();
    rendered_rows > 6 || tool_chain_active || stream_age >= Duration::from_millis(800)
}

fn rendered_height(lines: &[Line<'static>], wrap_width: usize) -> u16 {
    let wrap_width = wrap_width.max(1);
    lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(wrap_width))
        .sum::<usize>()
        .max(1) as u16
}

#[cfg_attr(not(test), allow(dead_code))]
fn split_semantic_blocks(text: &str) -> Vec<String> {
    if text.trim().is_empty() {
        return vec![String::new()];
    }

    let mut blocks = Vec::new();
    let mut current = Vec::new();
    let mut in_code_block = false;

    for raw_line in text.lines() {
        let trimmed = raw_line.trim_end();
        let is_fence = trimmed.starts_with("```");
        let is_list_item = is_list_item(trimmed);
        let is_heading = is_heading(trimmed);
        let is_blockquote = is_blockquote(trimmed);

        if is_fence {
            current.push(trimmed.to_string());
            if in_code_block {
                blocks.push(current.join("\n"));
                current.clear();
            }
            in_code_block = !in_code_block;
            continue;
        }

        if in_code_block {
            current.push(trimmed.to_string());
            continue;
        }

        if trimmed.is_empty() {
            if !current.is_empty() {
                blocks.push(current.join("\n"));
                current.clear();
            }
            continue;
        }

        if is_blockquote {
            if !current.is_empty() {
                blocks.push(current.join("\n"));
                current.clear();
            }
            blocks.push(trimmed.to_string());
            continue;
        }

        if is_heading {
            if !current.is_empty() {
                blocks.push(current.join("\n"));
                current.clear();
            }
            current.push(trimmed.to_string());
            continue;
        }

        if is_list_item {
            if !current.is_empty() {
                blocks.push(current.join("\n"));
                current.clear();
            }
            blocks.push(trimmed.to_string());
            continue;
        }

        current.push(trimmed.to_string());
    }

    if !current.is_empty() {
        blocks.push(current.join("\n"));
    }

    if blocks.is_empty() {
        blocks.push(text.to_string());
    }
    blocks
}

fn is_list_item(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("- ")
        || trimmed.starts_with("* ")
        || trimmed.starts_with("+ ")
        || trimmed
            .chars()
            .enumerate()
            .take_while(|(_, ch)| ch.is_ascii_digit())
            .last()
            .is_some_and(|(index, _)| trimmed[index + 1..].starts_with(". "))
}

fn is_heading(line: &str) -> bool {
    let trimmed = line.trim_start();
    let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
    hashes > 0 && trimmed[hashes..].starts_with(' ')
}

fn is_blockquote(line: &str) -> bool {
    line.trim_start().starts_with("> ")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ratatui::text::Line;

    use super::{CommitKind, CommittedSlice, ProjectionReducer};
    use crate::model::events::Event;

    #[test]
    fn reducer_keeps_only_tail_of_streaming_assistant_in_hud() {
        let mut reducer = ProjectionReducer::default();
        let events = vec![
            Event::UserTurn {
                id: "user-1".to_string(),
                text: "你好".to_string(),
            },
            Event::AssistantBlock {
                id: "assistant-1".to_string(),
                text: "第一段\n\n第二段\n\n第三段".to_string(),
                streaming: true,
            },
        ];
        let projection = reducer.reduce_events(&events, 68, 12, Duration::ZERO);
        assert_eq!(projection.commit_queue.len(), 1);
        assert!(matches!(
            projection.commit_queue[0].kind,
            CommitKind::UserTurn
        ));
        assert!(!projection.hud.detail_lines.is_empty());
        assert!(
            projection
                .hud
                .detail_lines
                .iter()
                .any(|line| line.to_string().contains("正在生成回复"))
        );
    }

    #[test]
    fn reducer_promotes_long_streaming_assistant_to_live_preview() {
        let mut reducer = ProjectionReducer::default();
        let events = vec![Event::AssistantBlock {
            id: "assistant-1".to_string(),
            text: "第一行\n第二行\n第三行\n第四行\n第五行\n第六行\n第七行".to_string(),
            streaming: true,
        }];
        let projection = reducer.reduce_events(&events, 36, 12, Duration::ZERO);
        assert_eq!(projection.hud.status_line, Some(Line::from("• Responding")));
        assert!(!projection.hud.live_preview_lines.is_empty());
        assert!(
            projection
                .hud
                .live_preview_lines
                .iter()
                .any(|line| line.to_string().contains("第七行"))
        );
    }

    #[test]
    fn completed_user_and_assistant_blocks_are_committed_in_order() {
        let mut reducer = ProjectionReducer::default();
        let projection = reducer.reduce_events(
            &[
                Event::UserTurn {
                    id: "user-1".to_string(),
                    text: "hello".to_string(),
                },
                Event::AssistantBlock {
                    id: "assistant-1".to_string(),
                    text: "world".to_string(),
                    streaming: false,
                },
            ],
            72,
            12,
            Duration::ZERO,
        );
        assert_eq!(projection.commit_queue.len(), 2);
        assert!(matches!(
            projection.commit_queue[0].kind,
            CommitKind::UserTurn
        ));
        assert!(matches!(
            projection.commit_queue[1].kind,
            CommitKind::AssistantBlock
        ));
        assert!(
            projection.commit_queue[0]
                .lines
                .last()
                .is_some_and(|line| line.to_string().is_empty())
        );
        assert!(
            projection.commit_queue[1]
                .lines
                .last()
                .is_some_and(|line| line.to_string().is_empty())
        );
    }

    #[test]
    fn committed_slice_height_counts_wrapped_wide_characters() {
        let slice =
            CommittedSlice::new(CommitKind::SystemNote, vec![Line::from("你好你好你好")], 4);
        assert_eq!(slice.height, 4);
    }

    #[test]
    fn split_semantic_blocks_keeps_heading_with_following_paragraph() {
        let blocks = super::split_semantic_blocks("前言\n\n## 标题\n正文第一行\n正文第二行");
        assert_eq!(blocks, vec!["前言", "## 标题\n正文第一行\n正文第二行"]);
    }

    #[test]
    fn split_semantic_blocks_treats_blockquote_as_its_own_block() {
        let blocks = super::split_semantic_blocks("第一段\n> 引用\n第二段");
        assert_eq!(blocks, vec!["第一段", "> 引用", "第二段"]);
    }

    #[test]
    fn tool_summary_commits_only_two_lines_with_artifact_reference() {
        let mut reducer = ProjectionReducer::default();
        let projection = reducer.reduce_events(
            &[Event::ToolSummary {
                id: "tool-1".to_string(),
                tool_name: "readFile".to_string(),
                summary: "line1 line1 line1 line1\nline2 line2 line2 line2\nline3".to_string(),
                artifact_path: Some("/tmp/result.txt".to_string()),
            }],
            30,
            12,
            Duration::ZERO,
        );
        assert_eq!(projection.commit_queue.len(), 1);
        assert!(projection.commit_queue[0].lines.len() <= 3);
        assert!(
            projection.commit_queue[0]
                .lines
                .iter()
                .any(|line| line.to_string().contains("/tmp/result.txt"))
        );
    }

    #[test]
    fn reset_clears_committed_once_and_frontier_progress() {
        let mut reducer = ProjectionReducer::default();
        let events = [Event::AssistantBlock {
            id: "assistant-1".to_string(),
            text: "第一段\n\n第二段".to_string(),
            streaming: true,
        }];
        let projection = reducer.reduce_events(&events, 72, 12, Duration::ZERO);
        assert!(projection.commit_queue.is_empty());

        reducer.reset();

        let projection = reducer.reduce_events(&events, 72, 12, Duration::ZERO);
        assert!(projection.commit_queue.is_empty());
    }
}
