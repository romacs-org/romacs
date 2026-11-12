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
    ROWS,
    ROWS_TESSELLATED,
    ROWS_REUSED,
    ROWS_UNMARKED,
    GLYPHS_WALKED,
    FONT_IDENTITIES,
    BYTES_UPLOADED,
    SUBMIT_INSTRUCTIONS,
];

/// Add N to the calling thread's count NAME.
pub(crate) fn add(name: &'static str, n: usize) {
    call_count::add(name, n as u64);
}
