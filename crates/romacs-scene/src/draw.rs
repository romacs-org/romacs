//! The flattened draw list: one placement of a scene as the GPU pass sees it.
//!
//! The scene core lowers a laid-out, motion-sampled tree into a [`DrawList`];
//! `romacs_renderer_wgpu::scene_graph` draws it. Plain data only: no GPU
//! handles, no VM values, no tree. Every geometric value is in logical
//! pixels of the target it belongs to, y down, origin at the target's top
//! left; the renderer multiplies by the device scale.
//!
//! Contract (the renderer refuses a list that breaks it, naming the item):
//! - `targets` is in dependency order: an item may sample only a target that
//!   appears earlier in the list, and the last target is the placement
//!   itself. A target group's texture is therefore complete before anything
//!   samples it, whatever its position among its siblings in the tree.
//! - Items are in paint order (tree order, then `z`), already composed:
//!   `transform`, `opacity` and `clip` include every ancestor's.
//! - Colours are sRGB-encoded with straight alpha, components in `0..=1`, so
//!   motion can interpolate them without quantising.

use kurbo::{Affine, BezPath, Point, Rect, Size, Vec2};
use std::sync::Arc;

/// A node's id inside its scene.
pub type NodeId = u32;

/// One placement's picture: offscreen targets, then the placement root last.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawList {
    pub targets: Vec<Target>,
}

/// Names a texture the pass renders: the placement root or a target group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TargetId(pub NodeId);

/// A texture the pass renders: a target group's subtree or the placement.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub id: TargetId,
    /// Logical size; the texture is `size × scale` device pixels.
    pub size: Size,
    pub clear: Color,
    pub items: Vec<Item>,
}

/// One drawable, with everything inherited from its ancestors folded in.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// The node this came from, for caches and diagnostics.
    pub node: NodeId,
    /// Node-local coordinates to target coordinates.
    pub transform: Affine,
    /// Product of the node's and its ancestors' opacity, `0..=1`.
    pub opacity: f32,
    /// Axis-aligned clip in target coordinates; `None` clips to the target.
    pub clip: Option<Rect>,
    pub paint: Paint,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    Rect(RectPaint),
    Path(PathPaint),
    Text(TextPaint),
    Cells(CellsPaint),
    Sample(SamplePaint),
}

/// A rounded rectangle; fill, stroke, shadow and glow are terms of one
/// signed distance field. Shadow and glow draw under the fill.
#[derive(Clone, Debug, PartialEq)]
pub struct RectPaint {
    pub rect: Rect,
    pub radius: f64,
    pub fill: Option<Color>,
    /// Drawn inside the edge, so the shape keeps its box.
    pub stroke: Option<Stroke>,
    pub shadow: Option<Shadow>,
    pub glow: Option<Glow>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    pub width: f64,
    pub color: Color,
}

/// A soft drop shadow: the shape grown by `spread`, moved by `offset`,
/// blurred by a Gaussian of deviation `blur / 2`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub offset: Vec2,
    pub blur: f64,
    pub spread: f64,
    pub color: Color,
}

/// Emitted light around the edge: an outer falloff over `radius` pixels,
/// strongest at the edge with `color`'s alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glow {
    pub radius: f64,
    pub color: Color,
}

/// An arbitrary path, tessellated on the CPU and filled with MSAA.
#[derive(Clone, Debug, PartialEq)]
pub struct PathPaint {
    pub path: Arc<BezPath>,
    /// Changes whenever `path`, `fill` or `stroke` changes; the renderer keeps
    /// the tessellation per `(node, revision)`.
    pub revision: u64,
    pub fill: Option<Fill>,
    pub stroke: Option<PathStroke>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fill {
    pub color: Color,
    pub rule: FillRule,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PathStroke {
    pub width: f64,
    pub color: Color,
    pub join: Join,
    pub cap: Cap,
    /// Alternating on/off lengths; empty is a solid stroke.
    pub dash: Vec<f64>,
    pub dash_offset: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Join {
    #[default]
    Miter,
    Round,
    Bevel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Cap {
    #[default]
    Butt,
    Round,
    Square,
}

/// One line of text, shaped and rasterised by the renderer's glyph atlas.
#[derive(Clone, Debug, PartialEq)]
pub struct TextPaint {
    /// Start of the baseline.
    pub origin: Point,
    pub text: Arc<str>,
    pub font: Font,
    pub color: Color,
}

/// A font as the wire carries it; the renderer resolves it to a realised
/// font through the frame's font table.
#[derive(Clone, Debug, PartialEq)]
pub struct Font {
    pub family: Arc<str>,
    /// Pixel size.
    pub size: f64,
    pub weight: u16,
    pub italic: bool,
}

/// A character grid (braille, half blocks, ramps) drawn with the glyph atlas.
#[derive(Clone, Debug, PartialEq)]
pub struct CellsPaint {
    /// Top-left corner of cell (0, 0).
    pub origin: Point,
    pub cols: u16,
    pub rows: u16,
    /// Advance of one cell.
    pub cell: Size,
    /// Row-major, `cols × rows` characters; a space draws nothing.
    pub glyphs: Arc<[char]>,
    /// Row-major palette indices, one per cell.
    pub shades: Arc<[u8]>,
    pub palette: Arc<[Color]>,
    pub font: Font,
}

/// A texture drawn into `rect`, clipped to `radius` rounded corners.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SamplePaint {
    pub source: Source,
    pub rect: Rect,
    pub radius: f64,
}

/// What a [`SamplePaint`] draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// A target earlier in the same draw list.
    Target(TargetId),
    /// An image in the host's image cache.
    Image(u32),
    /// A shader surface; a model's picture is installed as one too.
    Surface(u32),
}

/// sRGB-encoded colour with straight alpha.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Color = Color::rgba(0.0, 0.0, 0.0, 0.0);

    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb8(r: u8, g: u8, b: u8) -> Self {
        Self::rgba(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
    }

    /// `#rrggbb`, as the Omarchy palette files write colours.
    pub fn hex(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('#')?;
        let value = u32::from_str_radix(digits, 16).ok()?;
        (digits.len() == 6)
            .then(|| Self::rgb8((value >> 16) as u8, (value >> 8) as u8, value as u8))
    }

    pub const fn with_alpha(self, a: f32) -> Self {
        Self::rgba(self.r, self.g, self.b, a)
    }
}

#[cfg(test)]
mod tests {
    use super::Color;

    #[test]
    fn hex_reads_six_digit_colours_only() {
        assert_eq!(Color::hex("#d48a20"), Some(Color::rgb8(0xd4, 0x8a, 0x20)));
        assert_eq!(Color::hex("d48a20"), None);
        assert_eq!(Color::hex("#d48a2"), None);
        assert_eq!(Color::hex("#zzzzzz"), None);
    }
}
