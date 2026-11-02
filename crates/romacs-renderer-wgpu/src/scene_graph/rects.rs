//! Rect paints as SDF quads on the renderer's rounded-rect vertex.
//!
//! One `RectPaint` becomes up to three quads drawn in order: shadow, glow,
//! body. Each quad carries the node-local box, so the fragment shader
//! evaluates the same rounded-box distance for every term and any affine
//! transform keeps exact edges. Vertex field use:
//!
//! - `position`: the quad corner in target device pixels;
//! - `rect_min`/`rect_max`, `params` (`[stroke width, radius]`): node-local;
//! - `style_params`: `[term, term parameter, local x, local y]`;
//! - `color`: the term colour, `color2`: the body's stroke colour, both linear
//!   and premultiplied. The body composites its stroke over its fill first and
//!   applies the item's opacity (its term parameter) after; the other terms
//!   carry the opacity in their colour.

use super::color::linear_premultiplied;
use super::draw::{Color, RectPaint};
use crate::vertex::RoundedRectVertex;
use kurbo::{Affine, Point, Rect};

/// `style_params.x` values; the WGSL `TERM_*` constants match.
pub(super) const TERM_BODY: f32 = 0.0;
pub(super) const TERM_SHADOW: f32 = 1.0;
pub(super) const TERM_GLOW: f32 = 2.0;
pub(super) const TERM_SAMPLE: f32 = 3.0;

/// Gaussian tails beyond this many deviations are invisible in 8-bit colour.
const SHADOW_EXTENT_SIGMAS: f64 = 3.0;

/// Device pixels of anti-aliasing fringe around every quad.
const FRINGE_PX: f64 = 1.5;

/// The quads for one rect, in draw order.
pub(super) fn rect_quads(
    paint: &RectPaint,
    transform: Affine,
    opacity: f32,
    scale: f64,
) -> Vec<RoundedRectVertex> {
    let fringe = fringe(transform, scale);
    let radius = clamp_radius(paint.rect, paint.radius);
    let mut out = Vec::new();
    if let Some(shadow) = paint.shadow.filter(|s| s.color.a > 0.0) {
        let shape = (paint.rect + shadow.offset).inflate(shadow.spread, shadow.spread);
        let sigma = (shadow.blur / 2.0).max(0.0);
        let term = Term {
            kind: TERM_SHADOW,
            param: sigma as f32,
            shape,
            radius: clamp_radius(shape, radius + shadow.spread.max(0.0)),
            stroke_width: 0.0,
            color: shadow.color,
            color2: Color::TRANSPARENT,
            extent: sigma * SHADOW_EXTENT_SIGMAS + fringe,
        };
        push_quad(&mut out, &term, transform, opacity, scale);
    }
    if let Some(glow) = paint.glow.filter(|g| g.color.a > 0.0 && g.radius > 0.0) {
        let term = Term {
            kind: TERM_GLOW,
            param: glow.radius as f32,
            shape: paint.rect,
            radius,
            stroke_width: 0.0,
            color: glow.color,
            color2: Color::TRANSPARENT,
            extent: glow.radius + fringe,
        };
        push_quad(&mut out, &term, transform, opacity, scale);
    }
    let stroke = paint.stroke.filter(|s| s.width > 0.0 && s.color.a > 0.0);
    if paint.fill.is_some_and(|c| c.a > 0.0) || stroke.is_some() {
        let term = Term {
            kind: TERM_BODY,
            param: opacity.clamp(0.0, 1.0),
            shape: paint.rect,
            radius,
            stroke_width: stroke.map_or(0.0, |s| s.width),
            color: paint.fill.unwrap_or(Color::TRANSPARENT),
            color2: stroke.map_or(Color::TRANSPARENT, |s| s.color),
            extent: fringe,
        };
        push_quad(&mut out, &term, transform, 1.0, scale);
    }
    out
}

/// The quad that draws a texture into `rect` with rounded corners;
/// `straight` marks a straight-alpha source for the shader to premultiply.
pub(super) fn sample_quad(
    rect: Rect,
    radius: f64,
    straight: bool,
    transform: Affine,
    opacity: f32,
    scale: f64,
) -> Vec<RoundedRectVertex> {
    let fringe = fringe(transform, scale);
    let term = Term {
        kind: TERM_SAMPLE,
        param: 0.0,
        shape: rect,
        radius: clamp_radius(rect, radius),
        stroke_width: 0.0,
        // The texture is premultiplied already; the tint only carries opacity.
        color: Color::rgba(1.0, 1.0, 1.0, 1.0),
        color2: Color::TRANSPARENT,
        extent: fringe,
    };
    let mut out = Vec::new();
    push_quad(&mut out, &term, transform, opacity, scale);
    let flag = if straight { 1.0 } else { 0.0 };
    for vertex in &mut out {
        vertex.color2 = [flag, 0.0, 0.0, 0.0];
    }
    out
}

struct Term {
    kind: f32,
    param: f32,
    shape: Rect,
    radius: f64,
    stroke_width: f64,
    color: Color,
    color2: Color,
    /// Node-local distance the quad reaches beyond `shape`.
    extent: f64,
}

fn push_quad(
    out: &mut Vec<RoundedRectVertex>,
    term: &Term,
    transform: Affine,
    opacity: f32,
    scale: f64,
) {
    let quad = term.shape.inflate(term.extent, term.extent);
    let corners = [
        Point::new(quad.x0, quad.y0),
        Point::new(quad.x1, quad.y0),
        Point::new(quad.x1, quad.y1),
        Point::new(quad.x0, quad.y1),
    ];
    let to_device = Affine::scale(scale) * transform;
    let vertex = |local: Point| {
        let device = to_device * local;
        RoundedRectVertex {
            position: [device.x as f32, device.y as f32],
            color: linear_premultiplied(term.color, opacity),
            rect_min: [term.shape.x0 as f32, term.shape.y0 as f32],
            rect_max: [term.shape.x1 as f32, term.shape.y1 as f32],
            params: [term.stroke_width as f32, term.radius as f32],
            style_params: [term.kind, term.param, local.x as f32, local.y as f32],
            color2: linear_premultiplied(term.color2, opacity),
        }
    };
    for index in [0, 1, 2, 0, 2, 3] {
        out.push(vertex(corners[index]));
    }
}

/// A radius larger than half the short side would fold the box inside out.
fn clamp_radius(rect: Rect, radius: f64) -> f64 {
    radius.clamp(0.0, rect.width().min(rect.height()).abs() / 2.0)
}

/// Node-local distance that covers [`FRINGE_PX`] device pixels in every
/// direction, so a squashed axis keeps its anti-aliasing.
fn fringe(transform: Affine, scale: f64) -> f64 {
    FRINGE_PX / (scale * scale_range(transform).0).max(f64::EPSILON)
}

/// The least and greatest factor `transform` scales a length by (the
/// singular values of its linear part).
pub(super) fn scale_range(transform: Affine) -> (f64, f64) {
    let [a, b, c, d, _, _] = transform.as_coeffs();
    let sum = a * a + b * b + c * c + d * d;
    let det = a * d - b * c;
    let spread = (sum * sum - 4.0 * det * det).max(0.0).sqrt();
    (
        ((sum - spread) / 2.0).max(0.0).sqrt(),
        ((sum + spread) / 2.0).sqrt(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene_graph::draw::{Glow, Shadow, Stroke};
    use kurbo::Vec2;

    fn card() -> RectPaint {
        RectPaint {
            rect: Rect::new(10.0, 20.0, 110.0, 70.0),
            radius: 8.0,
            fill: Some(Color::rgb8(10, 10, 10)),
            stroke: None,
            shadow: None,
            glow: None,
        }
    }

    #[test]
    fn a_plain_card_is_one_body_quad() {
        let quads = rect_quads(&card(), Affine::IDENTITY, 1.0, 1.0);
        assert_eq!(quads.len(), 6);
        assert!(quads.iter().all(|v| v.style_params[0] == TERM_BODY));
        assert_eq!(quads[0].rect_min, [10.0, 20.0]);
        assert_eq!(quads[0].params, [0.0, 8.0]);
    }

    #[test]
    fn shadow_and_glow_draw_under_the_body() {
        let mut paint = card();
        paint.shadow = Some(Shadow {
            offset: Vec2::new(0.0, 6.0),
            blur: 12.0,
            spread: 0.0,
            color: Color::rgba(0.0, 0.0, 0.0, 0.5),
        });
        paint.glow = Some(Glow {
            radius: 10.0,
            color: Color::rgb8(0xd4, 0x8a, 0x20),
        });
        paint.stroke = Some(Stroke {
            width: 1.0,
            color: Color::rgb8(0x75, 0x6b, 0x59),
        });
        let terms: Vec<f32> = rect_quads(&paint, Affine::IDENTITY, 1.0, 1.0)
            .chunks(6)
            .map(|q| q[0].style_params[0])
            .collect();
        assert_eq!(terms, [TERM_SHADOW, TERM_GLOW, TERM_BODY]);
    }

    #[test]
    fn the_shadow_quad_covers_its_gaussian_tail() {
        let mut paint = card();
        paint.fill = None;
        paint.shadow = Some(Shadow {
            offset: Vec2::new(4.0, 6.0),
            blur: 12.0,
            spread: 2.0,
            color: Color::rgba(0.0, 0.0, 0.0, 0.5),
        });
        let quad = rect_quads(&paint, Affine::IDENTITY, 1.0, 1.0);
        // shape = rect + offset grown by spread; the quad adds 3 sigma + fringe.
        assert_eq!(quad[0].rect_min, [12.0, 24.0]);
        let reach = 2.0 + 6.0 * SHADOW_EXTENT_SIGMAS + FRINGE_PX;
        assert_eq!(
            quad[0].position,
            [(14.0 - reach) as f32, (26.0 - reach) as f32]
        );
    }

    #[test]
    fn transform_and_scale_move_positions_but_not_the_local_box() {
        let transform = Affine::translate((5.0, 5.0)) * Affine::scale(2.0);
        let quad = rect_quads(&card(), transform, 1.0, 1.5);
        let fringe = FRINGE_PX / 3.0;
        let local = 10.0 - fringe;
        assert_eq!(quad[0].style_params[2], local as f32);
        assert_eq!(quad[0].position[0], ((5.0 + 2.0 * local) * 1.5) as f32);
        assert_eq!(quad[0].rect_min, [10.0, 20.0]);
    }

    #[test]
    fn scale_range_is_the_singular_values() {
        let (low, high) = scale_range(Affine::scale_non_uniform(4.0, 0.25));
        assert!((low - 0.25).abs() < 1e-12 && (high - 4.0).abs() < 1e-12);
        let (low, high) = scale_range(Affine::rotate(0.7) * Affine::scale(3.0));
        assert!((low - 3.0).abs() < 1e-9 && (high - 3.0).abs() < 1e-9);
        // A squashed axis widens the local fringe so it still covers device pixels.
        let squashed = rect_quads(&card(), Affine::scale_non_uniform(4.0, 0.25), 1.0, 1.0);
        assert_eq!(squashed[0].style_params[3], (20.0 - FRINGE_PX * 4.0) as f32);
    }

    #[test]
    fn the_body_applies_opacity_after_compositing_its_stroke() {
        let mut paint = card();
        paint.stroke = Some(Stroke {
            width: 2.0,
            color: Color::rgba(1.0, 0.0, 0.0, 0.5),
        });
        let body = rect_quads(&paint, Affine::IDENTITY, 0.5, 1.0);
        assert_eq!(body[0].style_params[1], 0.5);
        assert_eq!(body[0].color2[3], 0.5, "stroke colour keeps its own alpha");
    }

    #[test]
    fn invisible_terms_emit_nothing_and_radius_is_clamped() {
        let mut paint = card();
        paint.fill = Some(Color::TRANSPARENT);
        assert!(rect_quads(&paint, Affine::IDENTITY, 1.0, 1.0).is_empty());
        paint.fill = Some(Color::rgb8(1, 2, 3));
        paint.radius = 500.0;
        assert_eq!(
            rect_quads(&paint, Affine::IDENTITY, 1.0, 1.0)[0].params[1],
            25.0
        );
    }
}
