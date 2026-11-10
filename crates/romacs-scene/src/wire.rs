//! `VM2GFX01`: scenes and diffs as bytes (§2.7).
//!
//! Numeric opcodes, little-endian `f32`/`u32`, length-prefixed UTF-8, RGBA8
//! colours, resources as integer ids. No symbols and no object references,
//! so the host decodes with no VM. The decoder is total: any byte string
//! yields a message or an error naming the offset and the node being read.

use std::sync::Arc;

use kurbo::{BezPath, PathEl, Point};
use romacs_display_protocol::motion_spec::{
    AngularFrequency, DampingRatio, DecelerationSpec, MotionDuration, MotionSpec, SpringSpec,
    TweenSpec, UnitBezier,
};
use romacs_display_protocol::scroll_animation::TransitionEasing;

use crate::diff::{Diff, Op, Rev};
use crate::motion::{Animate, Repeat};
use crate::node::{
    Align, AnchorId, Camera, Cap, Cells, Channel, Color, Common, Dim, Direction, FillRule, Fit,
    Flex, Font, Glow, ImageNode, ImageSource, Join, Kind, ModelNode, NodeId, NodeSpec, PathNode,
    RectNode, SceneId, SceneRef, ShaderNode, Shadow, Stroke, TextNode, Transform,
};
use crate::prop::{Prop, Value};
use crate::scene::{MAX_DEPTH, MAX_NODES};

pub const MAGIC: &[u8; 8] = b"VM2GFX01";
/// At most this many bytes per message.
pub const MAX_MESSAGE: usize = 4 << 20;
/// The longest tween the wire carries, in seconds.
pub(crate) const MAX_MOTION_SECS: f32 = 86_400.0;

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Create { scene: SceneId, diff: Diff },
    Update { scene: SceneId, diff: Diff },
    Destroy { scene: SceneId },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WireError {
    pub offset: usize,
    /// The node being read or written, when there was one.
    pub node: Option<NodeId>,
    pub what: &'static str,
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "VM2GFX01: {} at byte {}", self.what, self.offset)?;
        self.node.map_or(Ok(()), |n| write!(f, " (node {})", n.0))
    }
}

impl std::error::Error for WireError {}

// ── Encoding ────────────────────────────────────────────────────────────

/// Encode a message; refuses one over [`MAX_MESSAGE`].
pub fn encode(message: &Message) -> Result<Vec<u8>, WireError> {
    let mut w = Writer::default();
    w.bytes(MAGIC);
    match message {
        Message::Create { scene, diff } => {
            w.u8(1);
            w.u32(scene.0);
            w.diff(diff)?;
        }
        Message::Update { scene, diff } => {
            w.u8(2);
            w.u32(scene.0);
            w.diff(diff)?;
        }
        Message::Destroy { scene } => {
            w.u8(3);
            w.u32(scene.0);
        }
    }
    if w.out.len() > MAX_MESSAGE {
        return Err(WireError {
            offset: MAX_MESSAGE,
            node: w.node,
            what: "message over 4 MiB",
        });
    }
    Ok(w.out)
}

#[derive(Default)]
struct Writer {
    out: Vec<u8>,
    node: Option<NodeId>,
}

impl Writer {
    fn fail(&self, what: &'static str) -> WireError {
        WireError {
            offset: self.out.len(),
            node: self.node,
            what,
        }
    }

    fn bytes(&mut self, b: &[u8]) {
        self.out.extend_from_slice(b);
    }

    fn u8(&mut self, v: u8) {
        self.out.push(v);
    }

    fn bool(&mut self, v: bool) {
        self.u8(u8::from(v));
    }

    fn u16(&mut self, v: u16) {
        self.bytes(&v.to_le_bytes());
    }

    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }

    fn len(&mut self, n: usize) -> Result<(), WireError> {
        let n = u32::try_from(n).map_err(|_| self.fail("length over u32"))?;
        self.u32(n);
        Ok(())
    }

    fn i32(&mut self, v: i32) {
        self.bytes(&v.to_le_bytes());
    }

    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }

    fn f32(&mut self, v: f32) {
        self.bytes(&v.to_le_bytes());
    }

    fn f32s(&mut self, v: &[f32]) {
        v.iter().for_each(|x| self.f32(*x));
    }

    fn str(&mut self, s: &str) -> Result<(), WireError> {
        self.len(s.len())?;
        self.bytes(s.as_bytes());
        Ok(())
    }

    fn color(&mut self, c: Color) {
        let q = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
        self.bytes(&[q(c.r), q(c.g), q(c.b), q(c.a)]);
    }

    fn floats(&mut self, v: &[f32]) -> Result<(), WireError> {
        self.len(v.len())?;
        v.iter().for_each(|x| self.f32(*x));
        Ok(())
    }

    fn diff(&mut self, d: &Diff) -> Result<(), WireError> {
        self.u64(d.base.0);
        self.len(d.ops.len())?;
        d.ops.iter().try_for_each(|op| self.op(op))
    }

    fn op(&mut self, op: &Op) -> Result<(), WireError> {
        match op {
            Op::Insert {
                parent,
                index,
                node,
            } => {
                self.u8(1);
                self.u32(parent.0);
                self.u32(*index);
                self.node(node)
            }
            Op::Remove { id } => {
                self.u8(2);
                self.u32(id.0);
                Ok(())
            }
            Op::Move { id, parent, index } => {
                self.u8(3);
                self.u32(id.0);
                self.u32(parent.0);
                self.u32(*index);
                Ok(())
            }
            Op::Set {
                id,
                prop,
                value,
                animate,
            } => {
                self.node = Some(*id);
                self.u8(4);
                self.u32(id.0);
                self.prop(*prop);
                match value {
                    None => self.u8(0),
                    Some(v) => {
                        self.u8(1);
                        self.value(v)?;
                    }
                }
                self.animate(animate.as_ref())
            }
        }
    }

    fn node(&mut self, n: &NodeSpec) -> Result<(), WireError> {
        self.node = Some(n.id);
        self.u32(n.id.0);
        self.kind(&n.kind)?;
        self.common(&n.common);
        self.len(n.children.len())?;
        n.children.iter().try_for_each(|c| self.node(c))
    }

    fn common(&mut self, c: &Common) {
        let t = &c.transform;
        self.f32s(&[
            t.x, t.y, t.rotate, t.scale_x, t.scale_y, t.origin_x, t.origin_y, c.opacity,
        ]);
        self.u8(u8::from(c.clip) | u8::from(c.visible) << 1);
        self.i32(c.z);
        self.dim(c.width);
        self.dim(c.height);
        self.f32(c.grow);
        self.anchor(c.anchor);
    }

    fn dim(&mut self, d: Dim) {
        match d {
            Dim::Auto => self.u8(0),
            Dim::Px(v) => {
                self.u8(1);
                self.f32(v);
            }
            Dim::Fraction(v) => {
                self.u8(2);
                self.f32(v);
            }
        }
    }

    fn anchor(&mut self, a: Option<AnchorId>) {
        match a {
            None => self.u8(0),
            Some(a) => {
                self.u8(1);
                self.u32(a.0);
            }
        }
    }

    fn flex(&mut self, f: Option<Flex>) {
        let Some(f) = f else { return self.u8(0) };
        self.u8(1);
        self.u8(f.direction as u8);
        self.f32(f.gap);
        self.f32s(&f.padding);
        self.u8(f.align as u8);
    }

    fn stroke(&mut self, s: Stroke) {
        self.color(s.color);
        self.f32(s.width);
    }

    fn font(&mut self, f: &Font) -> Result<(), WireError> {
        self.str(&f.family)?;
        self.f32(f.size as f32);
        self.u16(f.weight);
        self.bool(f.italic);
        Ok(())
    }

    fn scene_ref(&mut self, r: SceneRef) {
        self.u32(r.scene.0);
        self.u32(r.generation);
        self.u32(r.node.0);
    }

    fn image(&mut self, s: &ImageSource) -> Result<(), WireError> {
        match s {
            ImageSource::File(f) => {
                self.u8(0);
                self.str(f)
            }
            ImageSource::Scene(r) => {
                self.u8(1);
                self.scene_ref(*r);
                Ok(())
            }
        }
    }

    fn channels(&mut self, cs: &[Channel]) -> Result<(), WireError> {
        self.len(cs.len())?;
        for c in cs {
            match c {
                Channel::Local(n) => {
                    self.u8(0);
                    self.u32(n.0);
                }
                Channel::Scene(r) => {
                    self.u8(1);
                    self.scene_ref(*r);
                }
            }
        }
        Ok(())
    }

    fn path(&mut self, p: &BezPath) -> Result<(), WireError> {
        self.len(p.elements().len())?;
        for el in p.elements() {
            match *el {
                PathEl::MoveTo(a) => self.verb(0, &[a]),
                PathEl::LineTo(a) => self.verb(1, &[a]),
                PathEl::QuadTo(a, b) => self.verb(2, &[a, b]),
                PathEl::CurveTo(a, b, c) => self.verb(3, &[a, b, c]),
                PathEl::ClosePath => self.verb(4, &[]),
            }
        }
        Ok(())
    }

    fn verb(&mut self, verb: u8, points: &[Point]) {
        self.u8(verb);
        for pt in points {
            self.f32(pt.x as f32);
            self.f32(pt.y as f32);
        }
    }

    fn cells(&mut self, g: &Cells) -> Result<(), WireError> {
        self.u16(g.cols);
        self.u16(g.rows);
        let glyphs: String = g.glyphs.iter().collect();
        self.str(&glyphs)?;
        self.len(g.shades.len())?;
        self.bytes(&g.shades);
        self.len(g.palette.len())?;
        g.palette.iter().for_each(|c| self.color(*c));
        self.font(&g.font)
    }

    fn kind(&mut self, k: &Kind) -> Result<(), WireError> {
        match k {
            Kind::Group { flex, target } => {
                self.u8(1);
                self.bool(*target);
                self.flex(*flex);
            }
            Kind::Rect(r) => {
                self.u8(2);
                self.f32(r.radius);
                self.color(r.fill);
                self.stroke(r.stroke);
                self.color(r.shadow.color);
                self.f32s(&[r.shadow.blur, r.shadow.spread, r.shadow.dx, r.shadow.dy]);
                self.color(r.glow.color);
                self.f32(r.glow.radius);
            }
            Kind::Path(p) => {
                self.u8(3);
                self.path(&p.data)?;
                self.color(p.fill);
                self.u8(p.rule as u8);
                self.stroke(p.stroke);
                self.u8(p.join as u8);
                self.u8(p.cap as u8);
                self.floats(&p.dash)?;
                self.f32(p.dash_offset);
            }
            Kind::Text(t) => {
                self.u8(4);
                self.str(&t.text)?;
                self.font(&t.font)?;
                self.color(t.color);
                self.bool(t.wrap);
            }
            Kind::Image(i) => {
                self.u8(5);
                self.image(&i.source)?;
                self.u8(i.fit as u8);
            }
            Kind::Shader(s) => {
                self.u8(6);
                self.str(&s.source)?;
                self.floats(&s.uniforms)?;
                self.u16(s.fps);
                self.channels(&s.channels)?;
            }
            Kind::Model(m) => {
                self.u8(7);
                self.u32(m.mesh);
                self.f32s(&[m.camera.yaw, m.camera.pitch, m.camera.distance]);
            }
            Kind::Cells(g) => {
                self.u8(8);
                self.cells(g)?;
            }
        }
        Ok(())
    }

    fn prop(&mut self, p: Prop) {
        let (tag, index) = prop_tag(p);
        self.u8(tag);
        if let Some(i) = index {
            self.u8(i);
        }
    }

    fn value(&mut self, v: &Value) -> Result<(), WireError> {
        match v {
            Value::Num(x) => {
                self.u8(1);
                self.f32(*x);
            }
            Value::Color(c) => {
                self.u8(2);
                self.color(*c);
            }
            Value::Bool(b) => {
                self.u8(3);
                self.bool(*b);
            }
            Value::Dim(d) => {
                self.u8(4);
                self.dim(*d);
            }
            Value::Str(s) => {
                self.u8(5);
                self.str(s)?;
            }
            Value::Path(p) => {
                self.u8(6);
                self.path(p)?;
            }
            Value::Floats(f) => {
                self.u8(7);
                self.floats(f)?;
            }
            Value::Flex(f) => {
                self.u8(8);
                self.flex(*f);
            }
            Value::Anchor(a) => {
                self.u8(9);
                self.anchor(*a);
            }
            Value::Font(f) => {
                self.u8(10);
                self.font(f)?;
            }
            Value::Image(s) => {
                self.u8(11);
                self.image(s)?;
            }
            Value::Fit(f) => {
                self.u8(12);
                self.u8(*f as u8);
            }
            Value::Rule(r) => {
                self.u8(13);
                self.u8(*r as u8);
            }
            Value::Join(j) => {
                self.u8(14);
                self.u8(*j as u8);
            }
            Value::Cap(c) => {
                self.u8(15);
                self.u8(*c as u8);
            }
            Value::Channels(c) => {
                self.u8(16);
                self.channels(c)?;
            }
            Value::Cells(g) => {
                self.u8(17);
                self.cells(g)?;
            }
        }
        Ok(())
    }

    fn animate(&mut self, a: Option<&Animate>) -> Result<(), WireError> {
        match a {
            None => self.u8(0),
            Some(Animate::Motion { spec, repeat }) => {
                self.u8(1);
                self.spec(*spec)?;
                match repeat {
                    Repeat::Once => self.u8(0),
                    Repeat::Times { count, alternate } => {
                        self.u8(1);
                        self.u32(*count);
                        self.bool(*alternate);
                    }
                    Repeat::Forever { alternate } => {
                        self.u8(2);
                        self.bool(*alternate);
                    }
                }
            }
            Some(Animate::Drive { rate }) => {
                self.u8(2);
                self.f32(*rate);
            }
        }
        Ok(())
    }

    fn spec(&mut self, s: MotionSpec) -> Result<(), WireError> {
        match s {
            MotionSpec::Tween(t) => {
                self.u8(1);
                self.f32(t.duration.as_secs_f32());
                self.u8(easing_tag(t.easing).ok_or_else(|| self.fail("unknown easing"))?);
                match t.bezier {
                    None => self.u8(0),
                    Some(b) => {
                        self.u8(1);
                        self.f32s(&[b.x1, b.y1, b.x2, b.y2]);
                    }
                }
            }
            MotionSpec::Spring(s) => {
                self.u8(2);
                self.f32(s.omega.get());
                self.f32(s.damping.get());
            }
            MotionSpec::Deceleration(d) => {
                self.u8(3);
                self.f32(d.friction.get());
            }
            // Instant, and any variant added later, snaps.
            _ => self.u8(0),
        }
        Ok(())
    }
}

// ── Tags ────────────────────────────────────────────────────────────────

const PROPS: &[Prop] = &[
    Prop::X,
    Prop::Y,
    Prop::Rotate,
    Prop::ScaleX,
    Prop::ScaleY,
    Prop::OriginX,
    Prop::OriginY,
    Prop::Opacity,
    Prop::Z,
    Prop::Visible,
    Prop::Clip,
    Prop::Width,
    Prop::Height,
    Prop::Grow,
    Prop::Anchor,
    Prop::Flex,
    Prop::Target,
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
    Prop::PathData,
    Prop::FillRule,
    Prop::Join,
    Prop::Cap,
    Prop::Dash,
    Prop::DashOffset,
    Prop::Text,
    Prop::Font,
    Prop::Color,
    Prop::Wrap,
    Prop::Source,
    Prop::Fit,
    Prop::ShaderSource,
    Prop::Uniforms,
    Prop::Uniform(0),
    Prop::Fps,
    Prop::Channels,
    Prop::Mesh,
    Prop::CameraYaw,
    Prop::CameraPitch,
    Prop::CameraDistance,
    Prop::Cells,
];

/// A property's wire tag (its 1-based position in [`PROPS`]) and, for a
/// uniform, its slot.
fn prop_tag(p: Prop) -> (u8, Option<u8>) {
    let (key, index) = match p {
        Prop::Uniform(i) => (Prop::Uniform(0), Some(i)),
        p => (p, None),
    };
    let tag = PROPS
        .iter()
        .position(|q| *q == key)
        .expect("every prop has a tag");
    (tag as u8 + 1, index)
}

const EASINGS: &[TransitionEasing] = &[
    TransitionEasing::EaseOutQuad,
    TransitionEasing::EaseOutCubic,
    TransitionEasing::Spring,
    TransitionEasing::Linear,
    TransitionEasing::EaseInOutCubic,
    TransitionEasing::CubicBezier,
    TransitionEasing::EaseOutExpo,
];

fn easing_tag(e: TransitionEasing) -> Option<u8> {
    EASINGS.iter().position(|x| *x == e).map(|i| i as u8)
}

// ── Decoding ────────────────────────────────────────────────────────────

/// Decode one message.
pub fn decode(bytes: &[u8]) -> Result<Message, WireError> {
    let mut r = Reader {
        bytes,
        at: 0,
        node: None,
        nodes: 0,
    };
    if bytes.len() > MAX_MESSAGE {
        return Err(r.fail("message over 4 MiB"));
    }
    if r.take(8)? != MAGIC {
        return Err(r.fail("not VM2GFX01"));
    }
    let message = match r.u8()? {
        1 => Message::Create {
            scene: SceneId(r.u32()?),
            diff: r.diff()?,
        },
        2 => Message::Update {
            scene: SceneId(r.u32()?),
            diff: r.diff()?,
        },
        3 => Message::Destroy {
            scene: SceneId(r.u32()?),
        },
        _ => return Err(r.fail("unknown message")),
    };
    if r.at != bytes.len() {
        return Err(r.fail("trailing bytes"));
    }
    Ok(message)
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    node: Option<NodeId>,
    nodes: usize,
}

impl<'a> Reader<'a> {
    fn fail(&self, what: &'static str) -> WireError {
        WireError {
            offset: self.at,
            node: self.node,
            what,
        }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let end = self
            .at
            .checked_add(n)
            .filter(|e| *e <= self.bytes.len())
            .ok_or_else(|| self.fail("truncated"))?;
        let out = &self.bytes[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        Ok(self.take(N)?.try_into().expect("length checked"))
    }

    fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.array::<1>()?[0])
    }

    fn bool(&mut self) -> Result<bool, WireError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(self.fail("bad boolean")),
        }
    }

    fn u16(&mut self) -> Result<u16, WireError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    fn u32(&mut self) -> Result<u32, WireError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn i32(&mut self) -> Result<i32, WireError> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, WireError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn f32(&mut self) -> Result<f32, WireError> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    /// A count of items of at least `min_bytes` each, bounded by the bytes left.
    fn count(&mut self, min_bytes: usize) -> Result<usize, WireError> {
        let n = self.u32()? as usize;
        if n.saturating_mul(min_bytes.max(1)) > self.bytes.len() - self.at {
            return Err(self.fail("count exceeds message"));
        }
        Ok(n)
    }

    fn str(&mut self) -> Result<Arc<str>, WireError> {
        let n = self.count(1)?;
        std::str::from_utf8(self.take(n)?)
            .map(Arc::from)
            .map_err(|_| self.fail("invalid UTF-8"))
    }

    fn color(&mut self) -> Result<Color, WireError> {
        let [r, g, b, a] = self.array::<4>()?.map(|c| f32::from(c) / 255.0);
        Ok(Color::rgba(r, g, b, a))
    }

    fn floats(&mut self) -> Result<Arc<[f32]>, WireError> {
        let n = self.count(4)?;
        (0..n).map(|_| self.f32()).collect()
    }

    fn diff(&mut self) -> Result<Diff, WireError> {
        let base = Rev(self.u64()?);
        let n = self.count(5)?;
        let ops = (0..n).map(|_| self.op()).collect::<Result<_, _>>()?;
        Ok(Diff { base, ops })
    }

    fn op(&mut self) -> Result<Op, WireError> {
        match self.u8()? {
            1 => Ok(Op::Insert {
                parent: NodeId(self.u32()?),
                index: self.u32()?,
                node: self.node(0)?,
            }),
            2 => Ok(Op::Remove {
                id: NodeId(self.u32()?),
            }),
            3 => Ok(Op::Move {
                id: NodeId(self.u32()?),
                parent: NodeId(self.u32()?),
                index: self.u32()?,
            }),
            4 => {
                let id = NodeId(self.u32()?);
                self.node = Some(id);
                let prop = self.prop()?;
                let value = if self.bool()? {
                    Some(self.value()?)
                } else {
                    None
                };
                let animate = self.animate()?;
                Ok(Op::Set {
                    id,
                    prop,
                    value,
                    animate,
                })
            }
            _ => Err(self.fail("unknown op")),
        }
    }

    fn node(&mut self, depth: usize) -> Result<NodeSpec, WireError> {
        let id = NodeId(self.u32()?);
        self.node = Some(id);
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(self.fail("more than 65536 nodes"));
        }
        if depth > MAX_DEPTH {
            return Err(self.fail("nesting deeper than 256"));
        }
        let kind = self.kind()?;
        let common = self.common()?;
        let n = self.count(4)?;
        let children = (0..n)
            .map(|_| self.node(depth + 1))
            .collect::<Result<_, _>>()?;
        Ok(NodeSpec {
            id,
            kind,
            common,
            children,
        })
    }

    fn common(&mut self) -> Result<Common, WireError> {
        let [x, y, rotate, scale_x, scale_y, origin_x, origin_y, opacity] =
            [(); 8].try_map_f32(self)?;
        let flags = self.u8()?;
        if flags > 3 {
            return Err(self.fail("bad flags"));
        }
        Ok(Common {
            transform: Transform {
                x,
                y,
                rotate,
                scale_x,
                scale_y,
                origin_x,
                origin_y,
            },
            opacity,
            clip: flags & 1 != 0,
            visible: flags & 2 != 0,
            z: self.i32()?,
            width: self.dim()?,
            height: self.dim()?,
            grow: self.f32()?,
            anchor: self.anchor()?,
        })
    }

    fn dim(&mut self) -> Result<Dim, WireError> {
        match self.u8()? {
            0 => Ok(Dim::Auto),
            1 => Ok(Dim::Px(self.f32()?)),
            2 => Ok(Dim::Fraction(self.f32()?)),
            _ => Err(self.fail("bad dimension")),
        }
    }

    fn anchor(&mut self) -> Result<Option<AnchorId>, WireError> {
        Ok(if self.bool()? {
            Some(AnchorId(self.u32()?))
        } else {
            None
        })
    }

    fn flex(&mut self) -> Result<Option<Flex>, WireError> {
        if !self.bool()? {
            return Ok(None);
        }
        let direction = match self.u8()? {
            0 => Direction::Row,
            1 => Direction::Column,
            _ => return Err(self.fail("bad direction")),
        };
        let gap = self.f32()?;
        let padding = [(); 4].try_map_f32(self)?;
        let align = match self.u8()? {
            0 => Align::Stretch,
            1 => Align::Start,
            2 => Align::Center,
            3 => Align::End,
            _ => return Err(self.fail("bad align")),
        };
        Ok(Some(Flex {
            direction,
            gap,
            padding,
            align,
        }))
    }

    fn stroke(&mut self) -> Result<Stroke, WireError> {
        Ok(Stroke {
            color: self.color()?,
            width: self.f32()?,
        })
    }

    fn font(&mut self) -> Result<Font, WireError> {
        Ok(Font {
            family: self.str()?,
            size: f64::from(self.f32()?),
            weight: self.u16()?,
            italic: self.bool()?,
        })
    }

    fn scene_ref(&mut self) -> Result<SceneRef, WireError> {
        Ok(SceneRef {
            scene: SceneId(self.u32()?),
            generation: self.u32()?,
            node: NodeId(self.u32()?),
        })
    }

    fn image(&mut self) -> Result<ImageSource, WireError> {
        match self.u8()? {
            0 => Ok(ImageSource::File(self.str()?)),
            1 => Ok(ImageSource::Scene(self.scene_ref()?)),
            _ => Err(self.fail("bad image source")),
        }
    }

    fn fit(&mut self) -> Result<Fit, WireError> {
        match self.u8()? {
            0 => Ok(Fit::Contain),
            1 => Ok(Fit::Cover),
            2 => Ok(Fit::Fill),
            3 => Ok(Fit::None),
            _ => Err(self.fail("bad fit")),
        }
    }

    fn rule(&mut self) -> Result<FillRule, WireError> {
        match self.u8()? {
            0 => Ok(FillRule::NonZero),
            1 => Ok(FillRule::EvenOdd),
            _ => Err(self.fail("bad fill rule")),
        }
    }

    fn join(&mut self) -> Result<Join, WireError> {
        match self.u8()? {
            0 => Ok(Join::Miter),
            1 => Ok(Join::Round),
            2 => Ok(Join::Bevel),
            _ => Err(self.fail("bad join")),
        }
    }

    fn cap(&mut self) -> Result<Cap, WireError> {
        match self.u8()? {
            0 => Ok(Cap::Butt),
            1 => Ok(Cap::Round),
            2 => Ok(Cap::Square),
            _ => Err(self.fail("bad cap")),
        }
    }

    fn channels(&mut self) -> Result<Arc<[Channel]>, WireError> {
        let n = self.count(5)?;
        (0..n)
            .map(|_| match self.u8()? {
                0 => Ok(Channel::Local(NodeId(self.u32()?))),
                1 => Ok(Channel::Scene(self.scene_ref()?)),
                _ => Err(self.fail("bad channel")),
            })
            .collect()
    }

    fn path(&mut self) -> Result<Arc<BezPath>, WireError> {
        let n = self.count(1)?;
        let mut path = BezPath::new();
        let mut started = false;
        for _ in 0..n {
            let verb = self.u8()?;
            let mut pt = || -> Result<Point, WireError> {
                Ok(Point::new(f64::from(self.f32()?), f64::from(self.f32()?)))
            };
            let el = match verb {
                0 => PathEl::MoveTo(pt()?),
                1 => PathEl::LineTo(pt()?),
                2 => PathEl::QuadTo(pt()?, pt()?),
                3 => PathEl::CurveTo(pt()?, pt()?, pt()?),
                4 => PathEl::ClosePath,
                _ => return Err(self.fail("bad path verb")),
            };
            // kurbo panics on a segment before any move-to.
            if !started && !matches!(el, PathEl::MoveTo(_)) {
                return Err(self.fail("path does not start with move-to"));
            }
            started = true;
            path.push(el);
        }
        Ok(Arc::new(path))
    }

    fn cells(&mut self) -> Result<Cells, WireError> {
        let cols = self.u16()?;
        let rows = self.u16()?;
        let glyphs: Arc<[char]> = self.str()?.chars().collect();
        let n = self.count(1)?;
        let shades: Arc<[u8]> = self.take(n)?.into();
        let n = self.count(4)?;
        let palette = (0..n).map(|_| self.color()).collect::<Result<_, _>>()?;
        Ok(Cells {
            cols,
            rows,
            glyphs,
            shades,
            palette,
            font: self.font()?,
        })
    }

    fn kind(&mut self) -> Result<Kind, WireError> {
        Ok(match self.u8()? {
            1 => {
                let target = self.bool()?;
                Kind::Group {
                    flex: self.flex()?,
                    target,
                }
            }
            2 => {
                let radius = self.f32()?;
                let fill = self.color()?;
                let stroke = self.stroke()?;
                let color = self.color()?;
                let [blur, spread, dx, dy] = [(); 4].try_map_f32(self)?;
                Kind::Rect(RectNode {
                    radius,
                    fill,
                    stroke,
                    shadow: Shadow {
                        color,
                        blur,
                        spread,
                        dx,
                        dy,
                    },
                    glow: Glow {
                        color: self.color()?,
                        radius: self.f32()?,
                    },
                })
            }
            3 => Kind::Path(PathNode {
                data: self.path()?,
                fill: self.color()?,
                rule: self.rule()?,
                stroke: self.stroke()?,
                join: self.join()?,
                cap: self.cap()?,
                dash: self.floats()?,
                dash_offset: self.f32()?,
            }),
            4 => Kind::Text(TextNode {
                text: self.str()?,
                font: self.font()?,
                color: self.color()?,
                wrap: self.bool()?,
            }),
            5 => Kind::Image(ImageNode {
                source: self.image()?,
                fit: self.fit()?,
            }),
            6 => Kind::Shader(ShaderNode {
                source: self.str()?,
                uniforms: self.floats()?.to_vec(),
                fps: self.u16()?,
                channels: self.channels()?.to_vec(),
            }),
            7 => Kind::Model(ModelNode {
                mesh: self.u32()?,
                camera: Camera {
                    yaw: self.f32()?,
                    pitch: self.f32()?,
                    distance: self.f32()?,
                },
            }),
            8 => Kind::Cells(self.cells()?),
            _ => return Err(self.fail("unknown node kind")),
        })
    }

    fn prop(&mut self) -> Result<Prop, WireError> {
        let tag = self.u8()?;
        match PROPS.get(usize::from(tag).wrapping_sub(1)) {
            Some(Prop::Uniform(_)) => Ok(Prop::Uniform(self.u8()?)),
            Some(p) => Ok(*p),
            None => Err(self.fail("unknown property")),
        }
    }

    fn value(&mut self) -> Result<Value, WireError> {
        Ok(match self.u8()? {
            1 => Value::Num(self.f32()?),
            2 => Value::Color(self.color()?),
            3 => Value::Bool(self.bool()?),
            4 => Value::Dim(self.dim()?),
            5 => Value::Str(self.str()?),
            6 => Value::Path(self.path()?),
            7 => Value::Floats(self.floats()?),
            8 => Value::Flex(self.flex()?),
            9 => Value::Anchor(self.anchor()?),
            10 => Value::Font(self.font()?),
            11 => Value::Image(self.image()?),
            12 => Value::Fit(self.fit()?),
            13 => Value::Rule(self.rule()?),
            14 => Value::Join(self.join()?),
            15 => Value::Cap(self.cap()?),
            16 => Value::Channels(self.channels()?),
            17 => Value::Cells(self.cells()?),
            _ => return Err(self.fail("unknown value")),
        })
    }

    fn animate(&mut self) -> Result<Option<Animate>, WireError> {
        match self.u8()? {
            0 => Ok(None),
            1 => {
                let spec = self.spec()?;
                let repeat = match self.u8()? {
                    0 => Repeat::Once,
                    1 => Repeat::Times {
                        count: self.u32()?,
                        alternate: self.bool()?,
                    },
                    2 => Repeat::Forever {
                        alternate: self.bool()?,
                    },
                    _ => return Err(self.fail("bad repeat")),
                };
                Ok(Some(Animate::Motion { spec, repeat }))
            }
            2 => Ok(Some(Animate::Drive { rate: self.f32()? })),
            _ => Err(self.fail("bad motion")),
        }
    }

    fn spec(&mut self) -> Result<MotionSpec, WireError> {
        let bad = |r: &Self| r.fail("invalid motion parameter");
        Ok(match self.u8()? {
            0 => MotionSpec::Instant,
            1 => {
                // `Duration::from_secs_f32` panics past its range; a day
                // is longer than any motion.
                let secs = Some(self.f32()?).filter(|s| (0.0..=MAX_MOTION_SECS).contains(s));
                let duration = secs
                    .and_then(|s| MotionDuration::from_secs_f32(s).ok())
                    .ok_or_else(|| bad(self))?;
                let easing = *EASINGS
                    .get(usize::from(self.u8()?))
                    .ok_or_else(|| self.fail("unknown easing"))?;
                let bezier = if self.bool()? {
                    let [x1, y1, x2, y2] = [(); 4].try_map_f32(self)?;
                    if ![x1, y1, x2, y2].iter().all(|v| v.is_finite()) {
                        return Err(bad(self));
                    }
                    Some(UnitBezier::new(x1, y1, x2, y2))
                } else {
                    None
                };
                MotionSpec::Tween(TweenSpec {
                    duration,
                    easing,
                    bezier,
                })
            }
            2 => MotionSpec::Spring(SpringSpec {
                omega: AngularFrequency::new(self.f32()?).map_err(|_| bad(self))?,
                damping: DampingRatio::new(self.f32()?).map_err(|_| bad(self))?,
            }),
            3 => MotionSpec::Deceleration(DecelerationSpec {
                friction: AngularFrequency::new(self.f32()?).map_err(|_| bad(self))?,
            }),
            _ => return Err(self.fail("unknown motion spec")),
        })
    }
}

/// Read `N` consecutive `f32`s.
trait TryMapF32<const N: usize> {
    fn try_map_f32(self, r: &mut Reader<'_>) -> Result<[f32; N], WireError>;
}

impl<const N: usize> TryMapF32<N> for [(); N] {
    fn try_map_f32(self, r: &mut Reader<'_>) -> Result<[f32; N], WireError> {
        let mut out = [0.0; N];
        for v in &mut out {
            *v = r.f32()?;
        }
        Ok(out)
    }
}
