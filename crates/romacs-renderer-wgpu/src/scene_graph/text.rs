//! Text and cells paints through the frame's glyph atlas.
//!
//! Text takes the atlas paths buffer text takes. Simple scripts go by
//! grapheme cluster: a single character through
//! `WgpuGlyphAtlas::get_or_create_atlas`, a cluster of several (combining
//! marks, emoji sequences) through the composed path, which shapes it. A line
//! in a script whose letters join or reorder (Arabic, Hebrew, the Indic
//! scripts) is shaped whole through the composed path, as composition hands
//! such runs to the shaper in a buffer. The atlas must be at the pass's
//! device scale.

use super::batch::{Batch, Scissor};
use super::color::linear_premultiplied;
use super::draw::{CellsPaint, Color, Font, TextPaint};
use crate::glyph_atlas::{
    GlyphAtlasHandle, GlyphKey, SubpixelRequest, WgpuGlyphAtlas, glyph_font_identity,
};
use crate::vertex::GlyphVertex;
use cosmic_text::SubpixelBin;
use kurbo::{Affine, Point};
use romacs_display_protocol::face::Face;
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

/// A draw-list font realised for this frame: the face whose resolved font the
/// atlas has installed, plus the metrics the pass cannot ask the atlas for.
#[derive(Clone, Debug)]
pub struct SceneFont {
    pub face: Face,
    /// Baseline below the top of a line, logical pixels.
    pub ascent: f32,
    /// Advance of characters the atlas holds no raster for (spaces).
    pub space_advance: f32,
}

/// Resolves draw-list fonts to realised ones; the frame's font table at
/// wiring time, a pinned test font in tests.
pub type FontLookup<'a> = dyn Fn(&Font) -> Option<SceneFont> + 'a;

pub(super) struct GlyphContext<'a> {
    pub atlas: &'a mut WgpuGlyphAtlas,
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub scale: f64,
}

/// Lay one line of text along its baseline.
pub(super) fn emit_text(
    cx: &mut GlyphContext<'_>,
    font: &SceneFont,
    paint: &TextPaint,
    transform: Affine,
    opacity: f32,
    scissor: Scissor,
    batch: &mut Batch,
) {
    let color = linear_premultiplied(paint.color, opacity);
    let size = paint.font.size;
    let space = f64::from(font.space_advance);
    let mut emit = |cx: &mut GlyphContext<'_>, text: &str, pen: Point| {
        emit_cluster(cx, font, size, text, pen, color, transform, scissor, batch)
    };
    let mut pen = paint.origin;
    if shapes_whole_line(&paint.text) {
        if emit(cx, &paint.text, pen).is_some() {
            return;
        }
        // A line too long for one atlas raster: shape it word by word, so
        // letters still join and the line stays visible; words then follow
        // in logical order.
        for word in paint.text.split(' ') {
            pen.x += emit(cx, word, pen).unwrap_or(0.0) + space;
        }
        return;
    }
    for cluster in paint.text.graphemes(true) {
        pen.x += emit(cx, cluster, pen).unwrap_or(space);
    }
}

/// Draw a character grid: one glyph per cell on the row's baseline.
pub(super) fn emit_cells(
    cx: &mut GlyphContext<'_>,
    font: &SceneFont,
    paint: &CellsPaint,
    transform: Affine,
    opacity: f32,
    scissor: Scissor,
    batch: &mut Batch,
) {
    let cols = usize::from(paint.cols);
    let cells = cols * usize::from(paint.rows);
    for (index, ch) in paint.glyphs.iter().take(cells).enumerate() {
        if *ch == ' ' {
            continue;
        }
        let shade = paint.shades.get(index).copied().unwrap_or(0);
        let color = paint
            .palette
            .get(usize::from(shade))
            .copied()
            .unwrap_or(Color::TRANSPARENT);
        if color.a <= 0.0 {
            continue;
        }
        let (row, col) = (index / cols, index % cols);
        let baseline = Point::new(
            paint.origin.x + col as f64 * paint.cell.width,
            paint.origin.y + row as f64 * paint.cell.height + f64::from(font.ascent),
        );
        let color = linear_premultiplied(color, opacity);
        emit_cluster(
            cx,
            font,
            paint.font.size,
            ch.encode_utf8(&mut [0; 4]),
            baseline,
            color,
            transform,
            scissor,
            batch,
        );
    }
}

/// Emit one grapheme cluster (or a whole shaped line) with its pen at
/// `baseline` (node-local); returns its logical advance, or `None` when the
/// atlas has no raster for it (whitespace, or too large for a page).
fn emit_cluster(
    cx: &mut GlyphContext<'_>,
    font: &SceneFont,
    size: f64,
    cluster: &str,
    baseline: Point,
    color: [f32; 4],
    transform: Affine,
    scissor: Scissor,
    batch: &mut Batch,
) -> Option<f64> {
    let handles = rasters(cx, font, size, cluster);
    if handles.is_empty() {
        return None;
    }
    let to_device = Affine::scale(cx.scale) * transform;
    // Rasters are whole device pixels; place them on the pixel grid when the
    // transform allows it, as buffer text is, so they stay sharp.
    let pen = snap(to_device, to_device * baseline);
    for handle in &handles {
        batch.push_glyphs(handle.entry, scissor, quad(handle, pen, color, transform));
    }
    let advance = handles.iter().map(|h| h.advance_width).fold(0.0, f32::max);
    Some(f64::from(advance) / cx.scale)
}

/// The atlas rasters of one cluster; empty for whitespace or a failure.
fn rasters(
    cx: &mut GlyphContext<'_>,
    font: &SceneFont,
    size: f64,
    cluster: &str,
) -> Vec<GlyphAtlasHandle> {
    let size_bits = (size as f32).to_bits();
    let mut chars = cluster.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => {
            let key = GlyphKey {
                charcode: ch as u32,
                face_id: font.face.id,
                font_size_bits: size_bits,
                font_identity: glyph_font_identity(Some(&font.face)),
                x_bin: SubpixelBin::Zero,
                y_bin: SubpixelBin::Zero,
            };
            cx.atlas
                .get_or_create_atlas(
                    cx.device,
                    cx.queue,
                    &key,
                    Some(&font.face),
                    SubpixelRequest::Disabled,
                )
                .into_iter()
                .collect()
        }
        (Some(_), Some(_)) => cx
            .atlas
            .get_or_create_composed_atlas(
                cx.device,
                cx.queue,
                cluster,
                font.face.id,
                size_bits,
                Some(&font.face),
                SubpixelBin::Zero,
                SubpixelBin::Zero,
                SubpixelRequest::Disabled,
            )
            .unwrap_or_default(),
        (None, _) => Vec::new(),
    }
}

/// One raster's quad, in device pixels, with its pen at device `pen`.
fn quad(
    handle: &GlyphAtlasHandle,
    pen: Point,
    color: [f32; 4],
    transform: Affine,
) -> [GlyphVertex; 6] {
    let metrics = handle.entry.metrics();
    let rect = handle.entry.rect();
    let uv = handle.entry.uv();
    let x0 = pen.x + f64::from(metrics.bearing_x);
    let y0 = pen.y - f64::from(metrics.bearing_y);
    let (w, h) = (f64::from(rect.width()), f64::from(rect.height()));
    // Offsets from the pen follow the transform's linear part, so rotated
    // text stays rotated.
    let linear = linear_part(transform);
    let corner = |dx: f64, dy: f64| {
        let offset = linear * Point::new(dx - pen.x, dy - pen.y);
        [(pen.x + offset.x) as f32, (pen.y + offset.y) as f32]
    };
    let (u0, v0, u1, v1) = (uv.min()[0], uv.min()[1], uv.max()[0], uv.max()[1]);
    let v = |position, tex_coords| GlyphVertex {
        position,
        tex_coords,
        color,
    };
    let tl = v(corner(x0, y0), [u0, v0]);
    let tr = v(corner(x0 + w, y0), [u1, v0]);
    let br = v(corner(x0 + w, y0 + h), [u1, v1]);
    let bl = v(corner(x0, y0 + h), [u0, v1]);
    [tl, tr, br, tl, br, bl]
}

/// Whether `text` must be shaped as one line: it has letters that join or
/// reorder in context, or an explicit bidi control. Scripts drawn letter by
/// letter keep the per-cluster path and its atlas reuse.
fn shapes_whole_line(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c, '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            || !matches!(
                c.script(),
                Script::Common
                    | Script::Inherited
                    | Script::Unknown
                    | Script::Latin
                    | Script::Greek
                    | Script::Cyrillic
                    | Script::Armenian
                    | Script::Georgian
                    | Script::Han
                    | Script::Hiragana
                    | Script::Katakana
                    | Script::Bopomofo
                    | Script::Hangul
                    | Script::Braille
            )
    })
}

/// `transform` without its translation, as a mapping of device offsets.
fn linear_part(transform: Affine) -> Affine {
    let [a, b, c, d, _, _] = transform.as_coeffs();
    Affine::new([a, b, c, d, 0.0, 0.0])
}

fn snap(to_device: Affine, point: Point) -> Point {
    let [a, b, c, d, _, _] = to_device.as_coeffs();
    if b == 0.0 && c == 0.0 && a > 0.0 && d > 0.0 {
        Point::new(point.x.round(), point.y.round())
    } else {
        point
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joining_and_reordering_text_is_shaped_whole() {
        assert!(!shapes_whole_line("find-file C-x C-f"));
        assert!(!shapes_whole_line("λ ελληνικά 中文 かな ⣿⠿"));
        assert!(!shapes_whole_line("e\u{301}"));
        assert!(shapes_whole_line("open سلام"));
        assert!(shapes_whole_line("שלום"));
        assert!(shapes_whole_line("नमस्ते"));
        assert!(shapes_whole_line("a\u{200f}b"));
    }

    #[test]
    fn only_axis_aligned_transforms_snap() {
        let p = Point::new(10.4, 3.6);
        assert_eq!(
            snap(Affine::translate((0.3, 0.0)), p),
            Point::new(10.0, 4.0)
        );
        assert_eq!(snap(Affine::rotate(0.1), p), p);
        assert_eq!(snap(Affine::scale_non_uniform(-1.0, 1.0), p), p);
    }

    #[test]
    fn the_linear_part_drops_translation() {
        let t = Affine::translate((5.0, 6.0)) * Affine::scale(2.0);
        assert_eq!(linear_part(t) * Point::new(1.0, 1.0), Point::new(2.0, 2.0));
    }
}
