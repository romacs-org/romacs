//! Which character decides a grapheme cluster's font.
//!
//! Layout and rendering must make the same choice, so the policy lives here,
//! below both of them.

pub use romacs_layout_input::cluster::is_composition_joiner;

/// The character that decides which font a cluster resolves through.
///
/// Shared by the layout-side per-char font realization and the renderer's
/// rasterization so both threads make the same fallback decision:
/// - A cluster containing U+FE0F (emoji variation selector) requests EMOJI
///   presentation for the whole cluster, so the font is resolved the way an
///   emoji codepoint is — probe with a canonical emoji (U+1F600) to reach
///   the color emoji font (an emoji keycap is digit + U+FE0F + U+20E3; the
///   digit alone would pick a monochrome font).
/// - Otherwise the first glyph-bearing non-ASCII character, skipping
///   zero-width joiners/selectors that don't determine the font.
/// - `None` means the cluster is ASCII-only: the face's primary font applies.
pub fn representative_char_for_cluster(text: &str) -> Option<char> {
    if text.contains('\u{FE0F}') {
        return Some('\u{1F600}');
    }
    text.chars()
        .find(|&ch| !ch.is_ascii() && !is_composition_joiner(ch))
}
