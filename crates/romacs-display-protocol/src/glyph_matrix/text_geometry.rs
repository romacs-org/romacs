//! Where a row's visual TEXT_AREA glyphs start horizontally.
//!
//! Paint and cursor placement use the same origin, so both read it here.

use super::{GlyphArea, GlyphRow};
use crate::types::Rect;

/// The x of ROW's first text glyph inside TEXT_BOUNDS. A right-to-left row
/// is right-aligned by the width its non-padding glyphs use.
pub(crate) fn text_origin_x(row: &GlyphRow, text_bounds: Rect, char_width: f32) -> f32 {
    let char_width = char_width.max(1.0);
    let offset = if row.reversed_p {
        let used_width: f32 = row.glyphs[GlyphArea::Text.index()]
            .iter()
            .filter(|glyph| !glyph.padding)
            .map(|glyph| glyph.materialized_pixel_advance(char_width))
            .sum();
        (text_bounds.width - used_width).max(0.0)
    } else {
        row.pixel_x.max(0.0)
    };
    text_bounds.x + offset
}
