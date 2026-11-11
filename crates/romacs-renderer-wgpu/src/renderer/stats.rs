use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct GlyphRenderStats {
    pub total_frame_glyphs: usize,
    pub text_glyphs: usize,
    pub composed_glyphs: usize,
    pub unique_single_glyph_keys: usize,
    pub unique_composed_glyph_keys: usize,
    pub glyph_texture_uploads: usize,
    pub glyph_draw_calls: usize,
    pub glyph_bind_group_changes: usize,
    pub glyph_vertex_buffer_creations: usize,
    pub composed_glyph_draw_calls: usize,
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub page_evictions: usize,
    /// GPU vertex-buffer allocations this frame (arena growth events).
    /// Zero in steady state; nonzero only while arenas grow to high water.
    pub buffers_created: usize,
    /// Text rows tessellated from scratch this frame (includes bailed rows).
    pub rows_tessellated: usize,
    /// Text rows spliced verbatim from the row-reuse cache.
    pub rows_reused_verbatim: usize,
    /// Text rows spliced with an integral vertical shift.
    pub rows_reused_shifted: usize,
    /// Rows whose damage said reusable but a defensive reuse key failed.
    pub row_reuse_bails: usize,
}

struct GlyphStatsWindow {
    start: Instant,
    frames: usize,
    totals: GlyphRenderStats,
}

thread_local! {
    static GLYPH_STATS_WINDOW: RefCell<GlyphStatsWindow> = RefCell::new(GlyphStatsWindow {
        start: Instant::now(),
        frames: 0,
        totals: GlyphRenderStats::default(),
    });
}

impl GlyphRenderStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn log_if_enabled(&self) {
        if !stats_enabled() {
            return;
        }
        GLYPH_STATS_WINDOW.with_borrow_mut(|window| {
            window.frames = window.frames.saturating_add(1);
            window.totals.accumulate(self);
            let elapsed = window.start.elapsed();
            if elapsed >= Duration::from_secs(1) {
                window.totals.log_aggregate(window.frames, elapsed);
                window.start = Instant::now();
                window.frames = 0;
                window.totals = Self::default();
            }
        });
    }

    fn accumulate(&mut self, frame: &Self) {
        self.total_frame_glyphs = self
            .total_frame_glyphs
            .saturating_add(frame.total_frame_glyphs);
        self.text_glyphs = self.text_glyphs.saturating_add(frame.text_glyphs);
        self.composed_glyphs = self.composed_glyphs.saturating_add(frame.composed_glyphs);
        self.unique_single_glyph_keys = self
            .unique_single_glyph_keys
            .saturating_add(frame.unique_single_glyph_keys);
        self.unique_composed_glyph_keys = self
            .unique_composed_glyph_keys
            .saturating_add(frame.unique_composed_glyph_keys);
        self.glyph_texture_uploads = self
            .glyph_texture_uploads
            .saturating_add(frame.glyph_texture_uploads);
        self.glyph_draw_calls = self.glyph_draw_calls.saturating_add(frame.glyph_draw_calls);
        self.glyph_bind_group_changes = self
            .glyph_bind_group_changes
            .saturating_add(frame.glyph_bind_group_changes);
        self.glyph_vertex_buffer_creations = self
            .glyph_vertex_buffer_creations
            .saturating_add(frame.glyph_vertex_buffer_creations);
        self.composed_glyph_draw_calls = self
            .composed_glyph_draw_calls
            .saturating_add(frame.composed_glyph_draw_calls);
        self.cache_hits = self.cache_hits.saturating_add(frame.cache_hits);
        self.cache_misses = self.cache_misses.saturating_add(frame.cache_misses);
        self.page_evictions = self.page_evictions.saturating_add(frame.page_evictions);
        self.buffers_created = self.buffers_created.saturating_add(frame.buffers_created);
        self.rows_tessellated = self.rows_tessellated.saturating_add(frame.rows_tessellated);
        self.rows_reused_verbatim = self
            .rows_reused_verbatim
            .saturating_add(frame.rows_reused_verbatim);
        self.rows_reused_shifted = self
            .rows_reused_shifted
            .saturating_add(frame.rows_reused_shifted);
        self.row_reuse_bails = self.row_reuse_bails.saturating_add(frame.row_reuse_bails);
    }

    fn log_aggregate(&self, frames: usize, elapsed: Duration) {
        tracing::info!(
            "glyph-stats: total={} text={} composed={} unique_single_sum={} unique_composed_sum={} \
             uploads={} draws={} bind_changes={} vertex_bufs={} composed_draws={} hits={} misses={} evictions={} bufs_created={} rows_tess={} rows_reused={} rows_shifted={} row_bails={} frames={} interval_ms={}",
            self.total_frame_glyphs,
            self.text_glyphs,
            self.composed_glyphs,
            self.unique_single_glyph_keys,
            self.unique_composed_glyph_keys,
            self.glyph_texture_uploads,
            self.glyph_draw_calls,
            self.glyph_bind_group_changes,
            self.glyph_vertex_buffer_creations,
            self.composed_glyph_draw_calls,
            self.cache_hits,
            self.cache_misses,
            self.page_evictions,
            self.buffers_created,
            self.rows_tessellated,
            self.rows_reused_verbatim,
            self.rows_reused_shifted,
            self.row_reuse_bails,
            frames,
            elapsed.as_millis(),
        );
    }
}

pub(crate) fn stats_enabled() -> bool {
    static ENABLED: AtomicBool = AtomicBool::new(false);
    static INITIALIZED: AtomicBool = AtomicBool::new(false);
    if !INITIALIZED.load(Ordering::Relaxed) {
        let enabled = std::env::var("ROMACS_RENDER_STATS")
            .map(|v| v != "0" && !v.is_empty())
            .unwrap_or(false);
        ENABLED.store(enabled, Ordering::Relaxed);
        INITIALIZED.store(true, Ordering::Relaxed);
    }
    ENABLED.load(Ordering::Relaxed)
}
