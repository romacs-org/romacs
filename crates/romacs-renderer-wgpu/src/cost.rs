//! What a frame costs the renderer, counted where the work is done: the
//! names of the render thread's counts (`romacs_display_protocol::call_count`)
//! this crate adds to. The render thread reads them per drawn frame
//! (romacs-display-runtime `frame_cost`); they are plain adds, always on.
use romacs_display_protocol::call_count;

/// Text rows of the root frame's glyph passes: buffer rows and chrome rows.
pub const ROWS: &str = "render.rows";
/// Of those, rows tessellated from their glyphs.
pub const ROWS_TESSELLATED: &str = "render.rows-tessellated";
/// Of those, rows drawn from what an earlier frame tessellated.
pub const ROWS_REUSED: &str = "render.rows-reused";
/// Of the rows tessellated, those the pass had no damage record for: rows
/// reuse could not consider.
pub const ROWS_UNMARKED: &str = "render.rows-unmarked";
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
pub const COUNTS: [&str; 9] = [
    ROWS,
    ROWS_TESSELLATED,
    ROWS_REUSED,
    ROWS_UNMARKED,
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

/// Count a text pass that met ROWS rows and tessellated TESSELLATED of
/// them, UNMARKED of those for want of a damage record.
pub(crate) fn add_rows(rows: usize, tessellated: usize, unmarked: usize) {
    if FOR_CURSOR.get() {
        add(CURSOR_ROWS, tessellated);
        return;
    }
    add(ROWS, rows);
    add(ROWS_TESSELLATED, tessellated);
    add(ROWS_UNMARKED, unmarked);
}
