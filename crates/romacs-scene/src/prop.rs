//! Node properties: the keys a `set` op names, their values, and the typecheck.
//!
//! `get` and `set` are the only code that knows which kind carries which
//! property, so the diff validator, the motion sampler and the codec all go
//! through them.

use std::sync::Arc;

use kurbo::BezPath;

use crate::node::{
    AnchorId, Cap, Cells, Channel, Color, Dim, FillRule, Fit, Flex, Font, ImageSource, Join, Kind,
    Node,
};

/// A property key. The discriminant is the wire tag (§2.7).
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Prop {
    X,
    Y,
    Rotate,
    ScaleX,
    ScaleY,
    OriginX,
    OriginY,
    Opacity,
    Z,
    Visible,
    Clip,
    Width,
    Height,
    Grow,
    Anchor,
    Flex,
    Target,
    Radius,
    Fill,
    StrokeColor,
    StrokeWidth,
    ShadowColor,
    ShadowBlur,
    ShadowSpread,
    ShadowX,
    ShadowY,
    GlowColor,
    GlowRadius,
    PathData,
    FillRule,
    Join,
    Cap,
    Dash,
    DashOffset,
    Text,
    Font,
    Color,
    Wrap,
    Source,
    Fit,
    ShaderSource,
    Uniforms,
    Uniform(u8),
    Fps,
    Channels,
    Mesh,
    CameraYaw,
    CameraPitch,
    CameraDistance,
    Cells,
}

impl Prop {
    /// Properties in degrees: a drive wraps them to `[0, 360)`.
    #[must_use]
    pub const fn is_angle(self) -> bool {
        matches!(self, Self::Rotate | Self::CameraYaw | Self::CameraPitch)
    }

    /// Properties that are settable but never interpolated: counts, handles,
    /// and `grow`, which only layout reads and layout never samples motion.
    const fn is_discrete(self) -> bool {
        matches!(self, Self::Z | Self::Fps | Self::Mesh | Self::Grow)
    }
}

/// A property value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Num(f32),
    Color(Color),
    Bool(bool),
    Dim(Dim),
    Str(Arc<str>),
    Path(Arc<BezPath>),
    Floats(Arc<[f32]>),
    Flex(Option<Flex>),
    Anchor(Option<AnchorId>),
    Font(Font),
    Image(ImageSource),
    Fit(Fit),
    Rule(FillRule),
    Join(Join),
    Cap(Cap),
    Channels(Arc<[Channel]>),
    Cells(Cells),
}

impl Value {
    /// Whether a motion can carry this property between these two values.
    #[must_use]
    pub fn interpolates(prop: Prop, from: &Self, to: &Self) -> bool {
        !prop.is_discrete()
            && matches!(
                (from, to),
                (Self::Num(_), Self::Num(_))
                    | (Self::Color(_), Self::Color(_))
                    | (Self::Dim(Dim::Px(_)), Self::Dim(Dim::Px(_)))
            )
    }
}

/// Why a property could not be set.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PropError {
    /// The node's kind has no such property.
    NotOnKind,
    /// The value has the wrong type for the property.
    WrongType,
    /// A number is NaN, infinite or outside the property's range.
    OutOfRange,
}

/// The value of `prop` on `node`, or `None` when its kind has no such property.
#[must_use]
pub fn get(node: &Node, prop: Prop) -> Option<Value> {
    let c = &node.common;
    let t = &c.transform;
    let num = |v: f32| Some(Value::Num(v));
    match (prop, &node.kind) {
        (Prop::X, _) => num(t.x),
        (Prop::Y, _) => num(t.y),
        (Prop::Rotate, _) => num(t.rotate),
        (Prop::ScaleX, _) => num(t.scale_x),
        (Prop::ScaleY, _) => num(t.scale_y),
        (Prop::OriginX, _) => num(t.origin_x),
        (Prop::OriginY, _) => num(t.origin_y),
        (Prop::Opacity, _) => num(c.opacity),
        (Prop::Z, _) => num(c.z as f32),
        (Prop::Visible, _) => Some(Value::Bool(c.visible)),
        (Prop::Clip, _) => Some(Value::Bool(c.clip)),
        (Prop::Width, _) => Some(Value::Dim(c.width)),
        (Prop::Height, _) => Some(Value::Dim(c.height)),
        (Prop::Grow, _) => num(c.grow),
        (Prop::Anchor, _) => Some(Value::Anchor(c.anchor)),
        (Prop::Flex, Kind::Group { flex, .. }) => Some(Value::Flex(*flex)),
        (Prop::Target, Kind::Group { target, .. }) => Some(Value::Bool(*target)),
        (Prop::Radius, Kind::Rect(r)) => num(r.radius),
        (Prop::Fill, Kind::Rect(r)) => Some(Value::Color(r.fill)),
        (Prop::Fill, Kind::Path(p)) => Some(Value::Color(p.fill)),
        (Prop::StrokeColor, Kind::Rect(r)) => Some(Value::Color(r.stroke.color)),
        (Prop::StrokeColor, Kind::Path(p)) => Some(Value::Color(p.stroke.color)),
        (Prop::StrokeWidth, Kind::Rect(r)) => num(r.stroke.width),
        (Prop::StrokeWidth, Kind::Path(p)) => num(p.stroke.width),
        (Prop::ShadowColor, Kind::Rect(r)) => Some(Value::Color(r.shadow.color)),
        (Prop::ShadowBlur, Kind::Rect(r)) => num(r.shadow.blur),
        (Prop::ShadowSpread, Kind::Rect(r)) => num(r.shadow.spread),
        (Prop::ShadowX, Kind::Rect(r)) => num(r.shadow.dx),
        (Prop::ShadowY, Kind::Rect(r)) => num(r.shadow.dy),
        (Prop::GlowColor, Kind::Rect(r)) => Some(Value::Color(r.glow.color)),
        (Prop::GlowRadius, Kind::Rect(r)) => num(r.glow.radius),
        (Prop::PathData, Kind::Path(p)) => Some(Value::Path(p.data.clone())),
        (Prop::FillRule, Kind::Path(p)) => Some(Value::Rule(p.rule)),
        (Prop::Join, Kind::Path(p)) => Some(Value::Join(p.join)),
        (Prop::Cap, Kind::Path(p)) => Some(Value::Cap(p.cap)),
        (Prop::Dash, Kind::Path(p)) => Some(Value::Floats(p.dash.clone())),
        (Prop::DashOffset, Kind::Path(p)) => num(p.dash_offset),
        (Prop::Text, Kind::Text(x)) => Some(Value::Str(x.text.clone())),
        (Prop::Font, Kind::Text(x)) => Some(Value::Font(x.font.clone())),
        (Prop::Color, Kind::Text(x)) => Some(Value::Color(x.color)),
        (Prop::Wrap, Kind::Text(x)) => Some(Value::Bool(x.wrap)),
        (Prop::Source, Kind::Image(i)) => Some(Value::Image(i.source.clone())),
        (Prop::Fit, Kind::Image(i)) => Some(Value::Fit(i.fit)),
        (Prop::ShaderSource, Kind::Shader(s)) => Some(Value::Str(s.source.clone())),
        (Prop::Uniforms, Kind::Shader(s)) => Some(Value::Floats(s.uniforms.as_slice().into())),
        (Prop::Uniform(i), Kind::Shader(s)) => {
            s.uniforms.get(usize::from(i)).copied().map(Value::Num)
        }
        (Prop::Fps, Kind::Shader(s)) => num(f32::from(s.fps)),
        (Prop::Channels, Kind::Shader(s)) => Some(Value::Channels(s.channels.as_slice().into())),
        (Prop::Mesh, Kind::Model(m)) => num(m.mesh as f32),
        (Prop::CameraYaw, Kind::Model(m)) => num(m.camera.yaw),
        (Prop::CameraPitch, Kind::Model(m)) => num(m.camera.pitch),
        (Prop::CameraDistance, Kind::Model(m)) => num(m.camera.distance),
        (Prop::Cells, Kind::Cells(g)) => Some(Value::Cells(g.clone())),
        _ => None,
    }
}

/// Set `prop` on `node` to `value`, checking kind, type and range.
pub fn set(node: &mut Node, prop: Prop, value: Value) -> Result<(), PropError> {
    if get(node, prop).is_none() {
        return Err(PropError::NotOnKind);
    }
    check(prop, &value)?;
    let c = &mut node.common;
    let t = &mut c.transform;
    match (prop, &mut node.kind, value) {
        (Prop::X, _, Value::Num(v)) => t.x = v,
        (Prop::Y, _, Value::Num(v)) => t.y = v,
        (Prop::Rotate, _, Value::Num(v)) => t.rotate = v,
        (Prop::ScaleX, _, Value::Num(v)) => t.scale_x = v,
        (Prop::ScaleY, _, Value::Num(v)) => t.scale_y = v,
        (Prop::OriginX, _, Value::Num(v)) => t.origin_x = v,
        (Prop::OriginY, _, Value::Num(v)) => t.origin_y = v,
        (Prop::Opacity, _, Value::Num(v)) => c.opacity = v,
        (Prop::Z, _, Value::Num(v)) => c.z = v as i32,
        (Prop::Visible, _, Value::Bool(v)) => c.visible = v,
        (Prop::Clip, _, Value::Bool(v)) => c.clip = v,
        (Prop::Width, _, Value::Dim(v)) => c.width = v,
        (Prop::Height, _, Value::Dim(v)) => c.height = v,
        (Prop::Grow, _, Value::Num(v)) => c.grow = v,
        (Prop::Anchor, _, Value::Anchor(v)) => c.anchor = v,
        (Prop::Flex, Kind::Group { flex, .. }, Value::Flex(v)) => *flex = v,
        (Prop::Target, Kind::Group { target, .. }, Value::Bool(v)) => *target = v,
        (Prop::Radius, Kind::Rect(r), Value::Num(v)) => r.radius = v,
        (Prop::Fill, Kind::Rect(r), Value::Color(v)) => r.fill = v,
        (Prop::Fill, Kind::Path(p), Value::Color(v)) => p.fill = v,
        (Prop::StrokeColor, Kind::Rect(r), Value::Color(v)) => r.stroke.color = v,
        (Prop::StrokeColor, Kind::Path(p), Value::Color(v)) => p.stroke.color = v,
        (Prop::StrokeWidth, Kind::Rect(r), Value::Num(v)) => r.stroke.width = v,
        (Prop::StrokeWidth, Kind::Path(p), Value::Num(v)) => p.stroke.width = v,
        (Prop::ShadowColor, Kind::Rect(r), Value::Color(v)) => r.shadow.color = v,
        (Prop::ShadowBlur, Kind::Rect(r), Value::Num(v)) => r.shadow.blur = v,
        (Prop::ShadowSpread, Kind::Rect(r), Value::Num(v)) => r.shadow.spread = v,
        (Prop::ShadowX, Kind::Rect(r), Value::Num(v)) => r.shadow.dx = v,
        (Prop::ShadowY, Kind::Rect(r), Value::Num(v)) => r.shadow.dy = v,
        (Prop::GlowColor, Kind::Rect(r), Value::Color(v)) => r.glow.color = v,
        (Prop::GlowRadius, Kind::Rect(r), Value::Num(v)) => r.glow.radius = v,
        (Prop::PathData, Kind::Path(p), Value::Path(v)) => p.data = v,
        (Prop::FillRule, Kind::Path(p), Value::Rule(v)) => p.rule = v,
        (Prop::Join, Kind::Path(p), Value::Join(v)) => p.join = v,
        (Prop::Cap, Kind::Path(p), Value::Cap(v)) => p.cap = v,
        (Prop::Dash, Kind::Path(p), Value::Floats(v)) => p.dash = v,
        (Prop::DashOffset, Kind::Path(p), Value::Num(v)) => p.dash_offset = v,
        (Prop::Text, Kind::Text(x), Value::Str(v)) => x.text = v,
        (Prop::Font, Kind::Text(x), Value::Font(v)) => x.font = v,
        (Prop::Color, Kind::Text(x), Value::Color(v)) => x.color = v,
        (Prop::Wrap, Kind::Text(x), Value::Bool(v)) => x.wrap = v,
        (Prop::Source, Kind::Image(i), Value::Image(v)) => i.source = v,
        (Prop::Fit, Kind::Image(i), Value::Fit(v)) => i.fit = v,
        (Prop::ShaderSource, Kind::Shader(s), Value::Str(v)) => s.source = v,
        (Prop::Uniforms, Kind::Shader(s), Value::Floats(v)) => s.uniforms = v.to_vec(),
        (Prop::Uniform(i), Kind::Shader(s), Value::Num(v)) => s.uniforms[usize::from(i)] = v,
        (Prop::Fps, Kind::Shader(s), Value::Num(v)) => s.fps = v as u16,
        (Prop::Channels, Kind::Shader(s), Value::Channels(v)) => s.channels = v.to_vec(),
        (Prop::Mesh, Kind::Model(m), Value::Num(v)) => m.mesh = v as u32,
        (Prop::CameraYaw, Kind::Model(m), Value::Num(v)) => m.camera.yaw = v,
        (Prop::CameraPitch, Kind::Model(m), Value::Num(v)) => m.camera.pitch = v,
        (Prop::CameraDistance, Kind::Model(m), Value::Num(v)) => m.camera.distance = v,
        (Prop::Cells, Kind::Cells(g), Value::Cells(v)) => *g = v,
        _ => return Err(PropError::WrongType),
    }
    Ok(())
}

/// Type and range of a value for a property, independent of the node.
fn check(prop: Prop, value: &Value) -> Result<(), PropError> {
    let ok = |cond: bool| {
        if cond {
            Ok(())
        } else {
            Err(PropError::OutOfRange)
        }
    };
    match (prop, value) {
        (_, Value::Num(v)) if !v.is_finite() => Err(PropError::OutOfRange),
        (Prop::Opacity, Value::Num(v)) => ok((0.0..=1.0).contains(v)),
        (Prop::Z, Value::Num(v)) => ok(v.fract() == 0.0 && v.abs() <= 1e6),
        (Prop::Fps, Value::Num(v)) => ok(v.fract() == 0.0 && (0.0..=240.0).contains(v)),
        (Prop::Mesh, Value::Num(v)) => ok(v.fract() == 0.0 && (0.0..=16_777_216.0).contains(v)),
        (
            Prop::Grow
            | Prop::Radius
            | Prop::StrokeWidth
            | Prop::ShadowBlur
            | Prop::GlowRadius
            | Prop::CameraDistance,
            Value::Num(v),
        ) => ok(*v >= 0.0),
        (Prop::Width | Prop::Height, Value::Dim(d)) => ok(match d {
            Dim::Auto => true,
            Dim::Px(v) | Dim::Fraction(v) => v.is_finite() && *v >= 0.0,
        }),
        (Prop::Flex, Value::Flex(Some(f))) => ok(f.gap.is_finite()
            && f.gap >= 0.0
            && f.padding.iter().all(|p| p.is_finite() && *p >= 0.0)),
        (Prop::Dash | Prop::Uniforms, Value::Floats(v)) => {
            ok(v.iter().all(|x| x.is_finite())
                && (prop != Prop::Dash || v.iter().all(|x| *x >= 0.0)))
        }
        (Prop::Font, Value::Font(f)) => ok(f.size.is_finite() && f.size > 0.0),
        (Prop::Cells, Value::Cells(g)) => ok(cells_valid(g)),
        (_, Value::Color(c)) => ok([c.r, c.g, c.b, c.a].iter().all(|x| (0.0..=1.0).contains(x))),
        (Prop::PathData, Value::Path(p)) => ok(p
            .elements()
            .iter()
            .flat_map(|el| path_el_points(*el))
            .all(|pt| pt.x.is_finite() && pt.y.is_finite())),
        _ => Ok(()),
    }
}

fn path_el_points(el: kurbo::PathEl) -> Vec<kurbo::Point> {
    use kurbo::PathEl;
    match el {
        PathEl::MoveTo(a) | PathEl::LineTo(a) => vec![a],
        PathEl::QuadTo(a, b) => vec![a, b],
        PathEl::CurveTo(a, b, c) => vec![a, b, c],
        PathEl::ClosePath => vec![],
    }
}

/// A grid's shape agrees with its contents.
#[must_use]
pub(crate) fn cells_valid(g: &Cells) -> bool {
    let n = usize::from(g.cols) * usize::from(g.rows);
    g.glyphs.len() == n
        && (g.shades.is_empty() || g.shades.len() == n)
        && !g.palette.is_empty()
        && g.shades.iter().all(|s| usize::from(*s) < g.palette.len())
        && g.font.size.is_finite()
        && g.font.size > 0.0
}

/// Every property a kind carries, for validating a whole inserted node.
#[must_use]
pub(crate) fn props_of(kind: &Kind) -> Vec<Prop> {
    let common = [
        Prop::X,
        Prop::Y,
        Prop::Rotate,
        Prop::ScaleX,
        Prop::ScaleY,
        Prop::OriginX,
        Prop::OriginY,
        Prop::Opacity,
        Prop::Z,
        Prop::Width,
        Prop::Height,
        Prop::Grow,
    ];
    let own: &[Prop] = match kind {
        Kind::Group { .. } => &[Prop::Flex],
        Kind::Rect(_) => &[
            Prop::Radius,
            Prop::Fill,
            Prop::StrokeColor,
            Prop::StrokeWidth,
            Prop::ShadowColor,
            Prop::ShadowBlur,
            Prop::ShadowSpread,
            Prop::ShadowX,
            Prop::ShadowY,
            Prop::GlowColor,
            Prop::GlowRadius,
        ],
        Kind::Path(_) => &[
            Prop::PathData,
            Prop::Fill,
            Prop::StrokeColor,
            Prop::StrokeWidth,
            Prop::Dash,
            Prop::DashOffset,
        ],
        Kind::Text(_) => &[Prop::Font, Prop::Color],
        Kind::Image(_) => &[],
        Kind::Model(_) => &[
            Prop::Mesh,
            Prop::CameraYaw,
            Prop::CameraPitch,
            Prop::CameraDistance,
        ],
        Kind::Shader(_) => &[Prop::Uniforms, Prop::Fps],
        Kind::Cells(_) => &[Prop::Cells],
    };
    common.into_iter().chain(own.iter().copied()).collect()
}

/// Check every property of a freshly built node, as an insert must.
pub(crate) fn check_node(node: &Node) -> Result<(), (Prop, PropError)> {
    props_of(&node.kind)
        .into_iter()
        .filter_map(|prop| get(node, prop).map(|value| (prop, value)))
        .try_for_each(|(prop, value)| check(prop, &value).map_err(|e| (prop, e)))
}
