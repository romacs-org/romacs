//! What a frame costs the renderer, counted where the work is done: the
//! names of the render thread's counts (`romacs_display_protocol::call_count`)
//! this crate adds to. The render thread reads them per drawn frame
//! (romacs-display-runtime `frame_cost`); they are plain adds, always on.
use romacs_display_protocol::call_count;

/// Text rows tessellated by the root frame's glyph passes: buffer rows and
/// chrome rows.
pub const ROWS_TESSELLATED: &str = "render.rows-tessellated";
/// Text rows the root picture kept from an earlier frame instead.
pub const ROWS_REUSED: &str = "render.rows-reused";
/// Bands of rows the root picture moved, copied a distance away.
pub const BANDS_MOVED: &str = "render.bands-moved";
/// Rows tessellated to draw a cursor over the root picture: the glyphs
/// under it, which it paints in its own colours. Counted apart from the
/// rows above, which are the frame's.
pub const CURSOR_ROWS: &str = "render.cursor-rows";
/// Glyph records handed to glyph passes, each of which walks them several
/// times: the root frame's, a child frame's, a cursor cell's.
pub const GLYPHS_WALKED: &str = "render.glyphs-walked";
/// A glyph's font identity hashed (`WgpuGlyphAtlas::glyph_font_identity_for_char`).
pub const FONT_IDENTITIES: &str = "render.font-identities";
/// Bytes written to the GPU: vertex arenas and glyph atlas pages.
pub const BYTES_UPLOADED: &str = "render.bytes-uploaded";
/// Instructions inside `Queue::submit`: wgpu's own and the driver's.
pub const SUBMIT_INSTRUCTIONS: &str = "render.submit-instructions";

/// Every count above.
pub const COUNTS: [&str; 8] = [
    ROWS_TESSELLATED,
    ROWS_REUSED,
    BANDS_MOVED,
    CURSOR_ROWS,
    GLYPHS_WALKED,
    FONT_IDENTITIES,
    BYTES_UPLOADED,
    SUBMIT_INSTRUCTIONS,
];

/// Add N to the calling thread's count NAME.
pub(crate) fn add(name: &'static str, n: usize) {
    call_count::add(name, n as u64);
}

thread_local! {
    /// Whether the glyph pass running on this thread draws a cursor.
    static FOR_CURSOR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run WORK, a glyph pass that draws a cursor: its rows count as the
/// cursor's (`CURSOR_ROWS`).
pub(crate) fn for_cursor<T>(work: impl FnOnce() -> T) -> T {
    FOR_CURSOR.set(true);
    let value = work();
    FOR_CURSOR.set(false);
    value
}

/// Count a text pass that tessellated ROWS rows.
pub(crate) fn add_rows(rows: usize) {
    add(if FOR_CURSOR.get() { CURSOR_ROWS } else { ROWS_TESSELLATED }, rows);
}
