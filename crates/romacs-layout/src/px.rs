//! Numbers as layout uses them: pixel coordinates in `f32`, whole numbers
//! far below 2^24, which `f32` holds exactly; and counts of them.

/// N pixels.
#[expect(clippy::cast_precision_loss, reason = "screen coordinates are far below 2^24")]
pub(crate) fn of(n: u32) -> f32 { n as f32 }

/// N pixels, from an image's or a glyph's signed geometry.
#[expect(clippy::cast_precision_loss, reason = "screen coordinates are far below 2^24")]
pub(crate) fn signed(n: i64) -> f32 { n as f32 }

/// N cells or glyphs.
#[expect(clippy::cast_precision_loss, reason = "a row holds far fewer than 2^24 cells")]
pub(crate) fn count(n: usize) -> f32 { n as f32 }

/// V, a width computed in `f64` by GNU's formulas, as layout's `f32`.
#[expect(clippy::cast_possible_truncation, reason = "a width on screen is far below `f32::MAX`")]
pub(crate) fn narrow(v: f64) -> f32 { v as f32 }

/// V rounded to whole pixels.
#[expect(clippy::cast_possible_truncation, reason = "a pixel size on screen is far below `i32::MAX`")]
pub(crate) fn whole(v: f32) -> i32 { v.round() as i32 }

/// V cells rounded, none when negative.
#[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a count of cells on screen: small and not negative")]
pub(crate) fn cells(v: f32) -> usize { v.round().max(0.0) as usize }
