//! Node kinds and their properties, as the host retains them.
//!
//! A node is plain data: a kind with its content, the properties every node
//! shares, and its children. Heavy payloads (strings, paths, cell grids) sit
//! behind `Arc` so a scene clones cheaply, which is what makes a diff
//! transactional: it is applied to a copy and the copy replaces the scene only
//! when every check passed.

use std::sync::Arc;

use kurbo::BezPath;

pub use crate::draw::{Cap, Color, FillRule, Font, Join};

/// A node id, scoped to its scene. Lisp chooses ids; the root is always 0.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodeId(pub u32);

impl NodeId {
    pub const ROOT: Self = Self(0);
}

/// A scene id. Ids may be reused after a scene is freed; the generation of a
/// [`SceneRef`] tells incarnations apart.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SceneId(pub u32);

/// A buffer-text anchor published by `romacs-layout` per window.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AnchorId(pub u32);

/// A reference into another scene: its root or one of its target groups.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SceneRef {
    pub scene: SceneId,
    pub generation: u32,
    pub node: NodeId,
}

/// A length in a layout style.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Dim {
    /// Intrinsic: the measured content, or what flex layout gives.
    #[default]
    Auto,
    Px(f32),
    /// A fraction of the parent's box (`1.0` is the whole parent).
    Fraction(f32),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Direction {
    #[default]
    Row,
    Column,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Align {
    #[default]
    Stretch,
    Start,
    Center,
    End,
}

/// The flex container style of a `group`. A group without one places its
/// children by their transforms alone.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Flex {
    pub direction: Direction,
    pub gap: f32,
    /// Top, right, bottom, left.
    pub padding: [f32; 4],
    pub align: Align,
}

/// Translate, rotate and scale about an origin given as a fraction of the box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub x: f32,
    pub y: f32,
    /// Degrees, clockwise in screen space.
    pub rotate: f32,
    pub scale_x: f32,
    pub scale_y: f32,
    pub origin_x: f32,
    pub origin_y: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            rotate: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            origin_x: 0.5,
            origin_y: 0.5,
        }
    }
}

/// The properties every node carries.
#[derive(Clone, Debug, PartialEq)]
pub struct Common {
    pub transform: Transform,
    pub opacity: f32,
    /// Clip this node's subtree to its box.
    pub clip: bool,
    /// Stacking among siblings; ties keep tree order.
    pub z: i32,
    pub visible: bool,
    pub width: Dim,
    pub height: Dim,
    /// Flex grow factor inside a flex group.
    pub grow: f32,
    /// Position and size this node at a buffer-text anchor's box.
    pub anchor: Option<AnchorId>,
}

impl Default for Common {
    fn default() -> Self {
        Self {
            transform: Transform::default(),
            opacity: 1.0,
            clip: false,
            z: 0,
            visible: true,
            width: Dim::Auto,
            height: Dim::Auto,
            grow: 0.0,
            anchor: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stroke {
    pub color: Color,
    pub width: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shadow {
    pub color: Color,
    pub blur: f32,
    pub spread: f32,
    pub dx: f32,
    pub dy: f32,
}

/// The accent-coloured outer falloff of a rect.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Glow {
    pub color: Color,
    pub radius: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct RectNode {
    pub radius: f32,
    pub fill: Color,
    pub stroke: Stroke,
    pub shadow: Shadow,
    pub glow: Glow,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PathNode {
    /// Node-local coordinates.
    pub data: Arc<BezPath>,
    pub fill: Color,
    pub rule: FillRule,
    pub stroke: Stroke,
    pub join: Join,
    pub cap: Cap,
    /// Alternating on/off lengths; empty is solid.
    pub dash: Arc<[f32]>,
    pub dash_offset: f32,
}

/// The font a node uses when Lisp names none: the frame's default family.
#[must_use]
pub fn default_font() -> Font {
    Font {
        family: Arc::from(""),
        size: 14.0,
        weight: 400,
        italic: false,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextNode {
    pub text: Arc<str>,
    pub font: Font,
    pub color: Color,
    /// Wrap at the box width.
    pub wrap: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ImageSource {
    File(Arc<str>),
    /// Another scene's root or target group.
    Scene(SceneRef),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Fit {
    #[default]
    Contain,
    Cover,
    Fill,
    None,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageNode {
    pub source: ImageSource,
    pub fit: Fit,
}

/// What a shader channel samples: only a scene or a target group (§2.4).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Channel {
    Local(NodeId),
    Scene(SceneRef),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShaderNode {
    /// WGSL or GLSL `mainImage`.
    pub source: Arc<str>,
    /// Uniform slots, each animatable.
    pub uniforms: Vec<f32>,
    /// Frames per second when animated; 0 is static.
    pub fps: u16,
    pub channels: Vec<Channel>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// Degrees.
    pub yaw: f32,
    /// Degrees.
    pub pitch: f32,
    pub distance: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: 0.0,
            distance: 1.0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelNode {
    /// A handle in the host's model store.
    pub mesh: u32,
    pub camera: Camera,
}

/// A character grid drawn with the glyph atlas.
#[derive(Clone, Debug, PartialEq)]
pub struct Cells {
    pub cols: u16,
    pub rows: u16,
    /// Exactly `cols * rows` characters, row-major.
    pub glyphs: Arc<[char]>,
    /// Empty (every cell uses palette entry 0), or one palette index per cell.
    pub shades: Arc<[u8]>,
    /// At least one colour.
    pub palette: Arc<[Color]>,
    pub font: Font,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    Group { flex: Option<Flex>, target: bool },
    Rect(RectNode),
    Path(PathNode),
    Text(TextNode),
    Image(ImageNode),
    Shader(ShaderNode),
    Model(ModelNode),
    Cells(Cells),
}

impl Kind {
    #[must_use]
    pub const fn group() -> Self {
        Self::Group {
            flex: None,
            target: false,
        }
    }

    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Group { .. } => "group",
            Self::Rect(_) => "rect",
            Self::Path(_) => "path",
            Self::Text(_) => "text",
            Self::Image(_) => "image",
            Self::Shader(_) => "shader",
            Self::Model(_) => "model",
            Self::Cells(_) => "cells",
        }
    }

    #[must_use]
    pub const fn is_group(&self) -> bool {
        matches!(self, Self::Group { .. })
    }

    #[must_use]
    pub const fn flex(&self) -> Option<Flex> {
        match self {
            Self::Group { flex, .. } => *flex,
            _ => None,
        }
    }

    #[must_use]
    pub const fn is_target(&self) -> bool {
        matches!(self, Self::Group { target: true, .. })
    }
}

/// A node as the host retains it.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub kind: Kind,
    pub common: Common,
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    /// The scene revision that last changed this node's own properties.
    pub revision: u64,
}

/// A node as a diff inserts it: a subtree with its ids.
#[derive(Clone, Debug, PartialEq)]
pub struct NodeSpec {
    pub id: NodeId,
    pub kind: Kind,
    pub common: Common,
    pub children: Vec<NodeSpec>,
}

impl NodeSpec {
    #[must_use]
    pub fn new(id: u32, kind: Kind) -> Self {
        Self {
            id: NodeId(id),
            kind,
            common: Common::default(),
            children: Vec::new(),
        }
    }

    #[must_use]
    pub fn with(mut self, common: impl FnOnce(&mut Common)) -> Self {
        common(&mut self.common);
        self
    }

    #[must_use]
    pub fn child(mut self, child: Self) -> Self {
        self.children.push(child);
        self
    }

    /// Nodes in this subtree, itself included.
    #[must_use]
    pub fn count(&self) -> usize {
        1 + self.children.iter().map(Self::count).sum::<usize>()
    }
}
