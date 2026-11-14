//! The root frame's panes (DIVERGENCES.md VM2-PANE-CHROME), drawn over its
//! glyphs: each pane but the selected one dimmed toward the frame
//! background, its corners rounded into that background, and an edge one
//! device pixel wide round it.
//!
//! Owns: the device-pixel geometry of that chrome and its one draw. Must
//! not: move or animate a pane; the layout's rectangles are drawn as given.

use super::WgpuRenderer;
use crate::vertex::{RectVertex, RoundedRectVertex};
use romacs_display_protocol::{Color, FrameGlyphBuffer, Pane, Rect};

#[cfg(test)]
mod tests;

/// The rounded-rect shader's style for the outside of its box.
const OUTSIDE: f32 = 13.0;
/// Its solid style: with a border width, a ring inside the box's edge.
const RING: f32 = 0.0;

/// API pixel edge EDGE at SCALE in device pixels, rounded once: a
/// boundary two panes share lands on one device column for both.
pub(crate) fn device_edge(edge: f32, scale: f32) -> f32 {
    (edge * scale).round()
}

/// One corner square of a pane: the part of BOX outside its rounded
/// corner of RADIUS filled with the frame background, and the arc of its
/// EDGE, WIDTH logical pixels, drawn within SQUARE.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Corner {
    pub square: Rect,
    pub pane: Rect,
    pub radius: f32,
    pub edge: Option<(Color, f32)>,
}

/// What PANES draw at SCALE over a frame whose background is BACKGROUND,
/// in logical pixels on device-pixel boundaries: DIMS under the corners,
/// EDGES the straight runs of each edge, one device pixel wide.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PaneGeometry {
    pub dims: Vec<(Rect, Color)>,
    pub corners: Vec<Corner>,
    pub edges: Vec<(Rect, Color)>,
}

/// The rect between device edges LEFT..RIGHT and TOP..BOTTOM at SCALE.
fn between(left: f32, top: f32, right: f32, bottom: f32, scale: f32) -> Rect {
    Rect::new(left / scale, top / scale, (right - left) / scale, (bottom - top) / scale)
}

pub(crate) fn pane_geometry(panes: &[Pane], background: Color, scale: f32) -> PaneGeometry {
    let mut out = PaneGeometry::default();
    for pane in panes {
        let b = pane.bounds;
        let [l, t, r, bottom] = [b.x, b.y, b.x + b.width, b.y + b.height].map(|e| device_edge(e, scale));
        if r - l < 2.0 || bottom - t < 2.0 { continue; }
        let pane_box = between(l, t, r, bottom, scale);
        let radius = device_edge(pane.radius, scale).min(((r - l) / 2.0).floor()).min(((bottom - t) / 2.0).floor());
        if pane.dim > 0.0 {
            out.dims.push((pane_box, Color::new(background.r, background.g, background.b, pane.dim)));
        }
        if radius > 0.0 {
            let edge = pane.edge.map(|color| (color, 1.0 / scale));
            for (x, y) in [(l, t), (r - radius, t), (l, bottom - radius), (r - radius, bottom - radius)] {
                let square = between(x, y, x + radius, y + radius, scale);
                out.corners.push(Corner { square, pane: pane_box, radius: radius / scale, edge });
            }
        }
        if let Some(color) = pane.edge {
            // Square corners: the top and bottom runs take the corner pixels.
            let inset = radius.max(1.0);
            for rect in [
                between(l + radius, t, r - radius, t + 1.0, scale),
                between(l + radius, bottom - 1.0, r - radius, bottom, scale),
                between(l, t + inset, l + 1.0, bottom - inset, scale),
                between(r - 1.0, t + inset, r, bottom - inset, scale),
            ] {
                if rect.width > 0.0 && rect.height > 0.0 { out.edges.push((rect, color)); }
            }
        }
    }
    out
}

/// The six vertices of SQUARE drawn in STYLE over BOX's rounded-rect SDF.
fn corner_quad(out: &mut Vec<RoundedRectVertex>, corner: &Corner, color: Color, style: f32, border: f32) {
    let p = corner.pane;
    let v = |x: f32, y: f32| RoundedRectVertex {
        position: [x, y],
        color: [color.r, color.g, color.b, color.a],
        rect_min: [p.x, p.y],
        rect_max: [p.x + p.width, p.y + p.height],
        params: [border, corner.radius],
        style_params: [style, 0.0, 0.0, 0.0],
        color2: [0.0; 4],
    };
    let s = corner.square;
    let (x0, y0, x1, y1) = (s.x, s.y, s.x + s.width, s.y + s.height);
    out.extend([v(x0, y0), v(x1, y0), v(x0, y1), v(x1, y0), v(x1, y1), v(x0, y1)]);
}

/// The vertices of PANES's chrome: dims, edges, and the corners'
/// rounded-rect quads.
struct PaneVertices {
    dims: Vec<RectVertex>,
    edges: Vec<RectVertex>,
    rounded: Vec<RoundedRectVertex>,
}

impl WgpuRenderer {
    fn pane_vertices(&self, frame: &FrameGlyphBuffer) -> PaneVertices {
        let geometry = pane_geometry(&frame.panes, frame.background, self.scale_factor);
        let mut out = PaneVertices { dims: Vec::new(), edges: Vec::new(), rounded: Vec::new() };
        for (rect, color) in &geometry.dims {
            self.add_rect(&mut out.dims, rect.x, rect.y, rect.width, rect.height, color);
        }
        for (rect, color) in &geometry.edges {
            self.add_rect(&mut out.edges, rect.x, rect.y, rect.width, rect.height, color);
        }
        for corner in &geometry.corners {
            corner_quad(&mut out.rounded, corner, frame.background, OUTSIDE, 0.0);
            if let Some((color, width)) = corner.edge { corner_quad(&mut out.rounded, corner, color, RING, width); }
        }
        out
    }

    /// Draw VERTICES into RENDER_PASS with DRAW's uniforms: the dims, then
    /// the corners over them, then the edges. The number of quads.
    fn draw_pane_vertices(&mut self, render_pass: &mut wgpu::RenderPass<'_>, vertices: &PaneVertices,
                          draw: &super::draw::DrawParameters) -> usize {
        render_pass.set_bind_group(0, draw.binding(), &[]);
        self.draw_rect_vertex_layer(render_pass, &vertices.dims);
        if let Some(upload) = self.arenas.rounded.upload(&self.device, &self.queue, &vertices.rounded) {
            render_pass.set_pipeline(&self.pipelines.rounded_rect);
            render_pass.set_vertex_buffer(0, upload.buffer_slice());
            render_pass.draw(0..vertices.rounded.len() as u32, 0..1);
        }
        self.draw_rect_vertex_layer(render_pass, &vertices.edges);
        (vertices.dims.len() + vertices.edges.len() + vertices.rounded.len()) / 6
    }

    /// Draw FRAME's panes into RENDER_PASS, the root glyph pass, after its
    /// content, with DRAW the pass's uniforms: so the root picture, its
    /// retained copy and its GPU time include them. The number of quads.
    pub(super) fn draw_panes(&mut self, render_pass: &mut wgpu::RenderPass<'_>, frame: &FrameGlyphBuffer,
                             draw: &super::draw::DrawParameters) -> usize {
        if frame.panes.is_empty() { return 0; }
        let vertices = self.pane_vertices(frame);
        self.draw_pane_vertices(render_pass, &vertices, draw)
    }
}
