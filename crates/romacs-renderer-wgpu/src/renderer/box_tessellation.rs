//! Shared box-span tessellation for top-level and child-frame render paths.
//!
//! Layout owns box-run topology.  This module is the single renderer seam that
//! turns that topology into sharp/3D/rounded geometry, so child frames cannot
//! drift from the primary frame when a new edge policy is added.

use romacs_display_protocol::DeviceScale;
use romacs_display_protocol::face::{BoxType, Face};
use romacs_display_protocol::types::{Color, Rect};

use super::super::vertex::{RectVertex, RoundedRectVertex};
use super::WgpuRenderer;
use super::frame_pass::{BoxPaintPolicy, BoxSpan};

fn offset_clip(clip: Option<Rect>, offset_x: f32, offset_y: f32) -> Option<Rect> {
    clip.map(|clip| Rect {
        x: clip.x + offset_x,
        y: clip.y + offset_y,
        width: clip.width,
        height: clip.height,
    })
}

/// The top-left and bottom-right colours of a sharp box, as pgtkterm.c
/// draws them: a line in BOX_COLOR; a raised or pressed button's relief
/// (pgtk_setup_relief_colors) from the box colour given (`:color') else
/// the face's background, lightened by 1.2 and darkened by 0.6.
fn sharp_edge_colors(face: &Face, box_color: Color) -> (Color, Color) {
    let base = face.box_color.unwrap_or(face.background);
    let light = lighter_color(base, 1.2, 0x8000);
    let dark = lighter_color(base, 0.6, 0x4000);
    match face.box_type {
        BoxType::Raised3D => (light, dark),
        BoxType::Sunken3D => (dark, light),
        _ => (box_color, box_color),
    }
}

/// pgtkterm.c pgtk_compute_lighter_color: COLOR's 16-bit sRGB channels
/// times FACTOR, a dark colour (brightness under 48000) moved further by
/// a share of DELTA, and DELTA added when that changes nothing.
fn lighter_color(color: Color, factor: f64, delta: i64) -> Color {
    let channel = |c: f32| i64::from(Color::linear_component_to_srgb_u8(c)) * 257;
    let (r, g, b) = (channel(color.r), channel(color.g), channel(color.b));
    let scale = |c: i64| ((factor * c as f64) as i64).min(0xffff);
    let (mut nr, mut ng, mut nb) = (scale(r), scale(g), scale(b));
    let bright = (2 * r + 3 * g + b) / 6;
    if bright < 48000 {
        let dimness = 1.0 - bright as f64 / 48000.0;
        let min_delta = (delta as f64 * dimness * factor / 2.0) as i64;
        let shift = |c: i64| if factor < 1.0 { (c - min_delta).max(0) } else { (c + min_delta).min(0xffff) };
        (nr, ng, nb) = (shift(nr), shift(ng), shift(nb));
    }
    let pixel = |r: i64, g: i64, b: i64| ((r >> 8) << 16 | (g >> 8) << 8 | (b >> 8)) as u32;
    let mut new = pixel(nr, ng, nb);
    if new == pixel(r, g, b) {
        new = pixel((delta + r).min(0xffff), (delta + g).min(0xffff), (delta + b).min(0xffff));
    }
    Color { a: color.a, ..Color::from_pixel(new) }
}

impl WgpuRenderer {
    pub(super) fn append_required_box_background_fill_geometry(
        &self,
        vertices: &mut Vec<RectVertex>,
        span: &BoxSpan,
        offset_x: f32,
        offset_y: f32,
    ) {
        if !span.requires_background_fill || span.policy != BoxPaintPolicy::Sharp {
            return;
        }
        let Some(background) = span.bg.as_ref() else {
            return;
        };
        let start = vertices.len();
        self.add_rect(
            vertices,
            span.x + offset_x,
            span.y + offset_y,
            span.width,
            span.height,
            background,
        );
        super::pointer_override::clip_new_rect_vertices(
            vertices,
            start,
            offset_clip(span.clip, offset_x, offset_y).as_ref(),
        );
    }

    pub(super) fn append_rounded_box_fill_geometry(
        &self,
        vertices: &mut Vec<RoundedRectVertex>,
        span: &BoxSpan,
        face: &Face,
        offset_x: f32,
        offset_y: f32,
    ) {
        let Some(background) = span.bg.as_ref() else {
            return;
        };
        if face.box_corner_radius <= 0 {
            return;
        }
        let start = vertices.len();
        let radius = (face.box_corner_radius as f32)
            .min(span.height * 0.45)
            .min(span.width * 0.45);
        self.add_rounded_rect_with_box_vertical_edges(
            vertices,
            span.x + offset_x,
            span.y + offset_y,
            span.width,
            span.height,
            span.height.max(span.width),
            radius,
            background,
            span.box_vertical_edges,
        );
        super::pointer_override::clip_new_rounded_vertices(
            vertices,
            start,
            offset_clip(span.clip, offset_x, offset_y).as_ref(),
        );
    }

    /// Append one span's border and report whether it is animated.
    pub(super) fn append_box_border_geometry(
        &self,
        sharp: &mut Vec<RectVertex>,
        rounded: &mut Vec<RoundedRectVertex>,
        span: &BoxSpan,
        face: &Face,
        device_scale: DeviceScale,
        offset_x: f32,
        offset_y: f32,
    ) -> bool {
        let box_color = *face.box_color.as_ref().unwrap_or(&face.foreground);
        let border_width = face
            .box_line_width
            .logical_geometry(device_scale)
            .paint_thickness()
            .get();
        // `:line-width (VWIDTH . HWIDTH)': the left and right edges' own width.
        let side_width = face
            .box_vertical_line_width
            .map_or(border_width, |width| width.logical_geometry(device_scale).paint_thickness().get());
        let clip = offset_clip(span.clip, offset_x, offset_y);
        let x = span.x + offset_x;
        let y = span.y + offset_y;

        if face.box_corner_radius > 0 {
            let start = rounded.len();
            let radius = (face.box_corner_radius as f32)
                .min(span.height * 0.45)
                .min(span.width * 0.45);
            let color2 = face.box_color2.as_ref().unwrap_or(&box_color);
            self.add_rounded_rect_styled_with_box_vertical_edges(
                rounded,
                x,
                y,
                span.width,
                span.height,
                border_width,
                radius,
                &box_color,
                face.box_border_style.gnu_code(),
                face.box_border_speed,
                color2,
                span.box_vertical_edges,
            );
            super::pointer_override::clip_new_rounded_vertices(rounded, start, clip.as_ref());
            return face.box_border_style.is_fancy();
        }

        let start = sharp.len();
        let (top_left, bottom_right) = sharp_edge_colors(face, box_color);
        self.add_rect(sharp, x, y, span.width, border_width, &top_left);
        self.add_rect(
            sharp,
            x,
            y + span.height - border_width,
            span.width,
            border_width,
            &bottom_right,
        );
        if span.box_vertical_edges.owns_left() {
            self.add_rect(sharp, x, y, side_width, span.height, &top_left);
        }
        if span.box_vertical_edges.owns_right() {
            self.add_rect(
                sharp,
                x + span.width - side_width,
                y,
                side_width,
                span.height,
                &bottom_right,
            );
        }
        super::pointer_override::clip_new_rect_vertices(sharp, start, clip.as_ref());
        false
    }
}

#[cfg(test)]
mod relief_tests {
    use super::lighter_color;
    use romacs_display_protocol::Color;

    /// pgtk_compute_lighter_color's answers: black's relief is #4c4c4c
    /// above (the dark boost) and #404040 below (unchanged, so DELTA is
    /// added); a mid grey #808080 lightens to #b2b2b2 and darkens to #474747.
    #[test]
    fn relief_colours_are_gnu_s() {
        let srgb = |c: Color| [c.r, c.g, c.b].map(Color::linear_component_to_srgb_u8);
        assert_eq!(srgb(lighter_color(Color::from_pixel(0x000000), 1.2, 0x8000)), [0x4c; 3]);
        assert_eq!(srgb(lighter_color(Color::from_pixel(0x000000), 0.6, 0x4000)), [0x40; 3]);
        assert_eq!(srgb(lighter_color(Color::from_pixel(0x808080), 1.2, 0x8000)), [0xb2; 3]);
        assert_eq!(srgb(lighter_color(Color::from_pixel(0x808080), 0.6, 0x4000)), [0x47; 3]);
    }
}
