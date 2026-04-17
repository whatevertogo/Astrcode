use std::time::Duration;

use super::StreamRenderMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrappedLine {
    pub style: WrappedLineStyle,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrappedLineStyle {
    Plain,
    Muted,
    Divider,
    HeroBorder,
    HeroTitle,
    HeroBody,
    HeroMuted,
    HeroFeedTitle,
    Selection,
    PromptEcho,
    ThinkingLabel,
    ThinkingPreview,
    ThinkingBody,
    ToolLabel,
    ToolBody,
    Notice,
    ErrorText,
    FooterInput,
    FooterStatus,
    FooterHint,
    FooterKey,
    PaletteItem,
    PaletteSelected,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TranscriptRenderCache {
    pub width: u16,
    pub revision: u64,
    pub lines: Vec<WrappedLine>,
    pub selected_line_range: Option<(usize, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FooterRenderCache {
    pub width: u16,
    pub lines: Vec<WrappedLine>,
    pub cursor_col: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PaletteRenderCache {
    pub width: u16,
    pub lines: Vec<WrappedLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DirtyRegions {
    pub transcript: bool,
    pub footer: bool,
    pub palette: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RenderState {
    pub viewport_width: u16,
    pub viewport_height: u16,
    pub transcript_revision: u64,
    pub transcript_cache: TranscriptRenderCache,
    pub footer_cache: FooterRenderCache,
    pub palette_cache: PaletteRenderCache,
    pub dirty: DirtyRegions,
    pub frame_dirty: bool,
}

impl RenderState {
    pub fn set_viewport_size(&mut self, width: u16, height: u16) -> bool {
        if self.viewport_width == width && self.viewport_height == height {
            return false;
        }
        self.viewport_width = width;
        self.viewport_height = height;
        self.transcript_cache = TranscriptRenderCache::default();
        self.footer_cache = FooterRenderCache::default();
        self.palette_cache = PaletteRenderCache::default();
        self.dirty = DirtyRegions {
            transcript: true,
            footer: true,
            palette: true,
        };
        self.frame_dirty = true;
        true
    }

    pub fn update_transcript_cache(
        &mut self,
        width: u16,
        lines: Vec<WrappedLine>,
        selected_line_range: Option<(usize, usize)>,
    ) {
        self.transcript_cache = TranscriptRenderCache {
            width,
            revision: self.transcript_revision,
            lines,
            selected_line_range,
        };
        self.dirty.transcript = false;
    }

    pub fn invalidate_transcript_cache(&mut self) {
        self.transcript_revision = self.transcript_revision.saturating_add(1);
        self.transcript_cache = TranscriptRenderCache::default();
        self.mark_transcript_dirty();
    }

    pub fn update_footer_cache(&mut self, width: u16, lines: Vec<WrappedLine>, cursor_col: u16) {
        self.footer_cache = FooterRenderCache {
            width,
            lines,
            cursor_col,
        };
        self.dirty.footer = false;
    }

    pub fn invalidate_footer_cache(&mut self) {
        self.footer_cache = FooterRenderCache::default();
        self.mark_footer_dirty();
    }

    pub fn update_palette_cache(&mut self, width: u16, lines: Vec<WrappedLine>) {
        self.palette_cache = PaletteRenderCache { width, lines };
        self.dirty.palette = false;
    }

    pub fn invalidate_palette_cache(&mut self) {
        self.palette_cache = PaletteRenderCache::default();
        self.mark_palette_dirty();
    }

    pub fn mark_transcript_dirty(&mut self) {
        self.dirty.transcript = true;
        self.frame_dirty = true;
    }

    pub fn mark_footer_dirty(&mut self) {
        self.dirty.footer = true;
        self.frame_dirty = true;
    }

    pub fn mark_palette_dirty(&mut self) {
        self.dirty.palette = true;
        self.frame_dirty = true;
    }
    pub fn mark_all_dirty(&mut self) {
        self.dirty = DirtyRegions {
            transcript: true,
            footer: true,
            palette: true,
        };
        self.frame_dirty = true;
    }

    pub fn take_frame_dirty(&mut self) -> bool {
        std::mem::take(&mut self.frame_dirty)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamViewState {
    pub mode: StreamRenderMode,
    pub pending_chunks: usize,
    pub oldest_chunk_age: Duration,
}

impl Default for StreamViewState {
    fn default() -> Self {
        Self {
            mode: StreamRenderMode::Smooth,
            pending_chunks: 0,
            oldest_chunk_age: Duration::ZERO,
        }
    }
}

impl StreamViewState {
    pub fn update(
        &mut self,
        mode: StreamRenderMode,
        pending_chunks: usize,
        oldest_chunk_age: Duration,
    ) {
        self.mode = mode;
        self.pending_chunks = pending_chunks;
        self.oldest_chunk_age = oldest_chunk_age;
    }
}
