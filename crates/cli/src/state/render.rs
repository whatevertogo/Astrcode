use std::time::Duration;

use super::{StreamRenderMode, WrappedLine};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TranscriptRenderCache {
    pub width: u16,
    pub revision: u64,
    pub lines: Vec<WrappedLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RenderState {
    pub viewport_width: u16,
    pub viewport_height: u16,
    pub transcript_revision: u64,
    pub wrap_cache_revision: u64,
    pub transcript_cache: TranscriptRenderCache,
}

impl RenderState {
    pub fn set_viewport_size(&mut self, width: u16, height: u16) -> bool {
        if self.viewport_width == width && self.viewport_height == height {
            return false;
        }
        self.viewport_width = width;
        self.viewport_height = height;
        self.wrap_cache_revision = self.wrap_cache_revision.saturating_add(1);
        self.transcript_cache = TranscriptRenderCache::default();
        true
    }

    pub fn update_transcript_cache(&mut self, width: u16, lines: Vec<WrappedLine>) {
        self.transcript_cache = TranscriptRenderCache {
            width,
            revision: self.transcript_revision,
            lines,
        };
    }

    pub fn invalidate_transcript_cache(&mut self) {
        self.transcript_revision = self.transcript_revision.saturating_add(1);
        self.transcript_cache = TranscriptRenderCache::default();
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
