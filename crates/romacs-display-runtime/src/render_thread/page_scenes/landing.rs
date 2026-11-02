//! The *Romacs* landing as a scene: the mark as a lit, turning object (a
//! `shader` node, landing_mark.wgsl, reading the mark's distance field from
//! landing_field.wgsl) and the wordmark as a vector `path` under it, over
//! nothing: the placement is transparent where neither draws, so the page's
//! own background shows through at the frame's opacity.  Every motion is a property motion the host
//! samples: the mark's assembly, its turn and its light cycle are uniforms
//! with motions, the wordmark rises in on its opacity and position.  Under
//! reduced motion they all snap to their targets, which are the still frame.
//!
//! Pure: geometry in, a diff and WGSL out.  The mark and wordmark outlines
//! and where a view puts them come from romacs-logo, so this rendition and
//! the braille one sit in the same places.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use kurbo::{Affine, BezPath, PathEl, Rect, Shape, Size};
use romacs_display_protocol::motion_spec::{MotionDuration, MotionSpec, TweenSpec, UnitBezier};
use romacs_display_protocol::scroll_animation::TransitionEasing;
use romacs_scene::diff::{Diff, Op, Rev};
use romacs_scene::motion::{Animate, Policy, Repeat};
use romacs_scene::node::{Color, Dim, FillRule, Kind, NodeId, NodeSpec, PathNode, ShaderNode};
use romacs_scene::prop::{Prop, Value};

/// The mark's shader node.
pub(crate) const MARK: NodeId = NodeId(2);
pub(crate) const WORDMARK: NodeId = NodeId(3);

/// The mark's uniforms, as the shader reads them (landing_mark.wgsl).
pub(crate) const UNIFORMS: usize = 8;

/// The turn: yaw and pitch in degrees, each swinging between two values.
const YAW: (f32, f32) = (17.0, -17.0);
const PITCH: (f32, f32) = (-5.0, 9.0);
/// How far the wordmark rises as it fades in, in logical pixels.
const RISE: f64 = 14.0;

/// The margin around the mark the glow reaches into, in mark heights.
const MARGIN: f64 = 0.17;

/// The smallest mark worth drawing as an object, in logical pixels; a
/// smaller one stays in its braille cells.
const MIN_MARK: f64 = 48.0;

/// The distance field's surface size in logical pixels: its height.
const FIELD_HEIGHT: f64 = 640.0;

/// Where the landing draws in a block of text cells, in logical pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Geometry {
    /// The placement within the block.
    pub placement: Rect,
    /// The mark's shader box, in placement coordinates; the mark is this box
    /// less `margin` on every side.
    pub mark: Rect,
    pub margin: f64,
    /// The wordmark's box, in placement coordinates.
    pub word: Rect,
    /// The braille dot pitch: half a cell wide, a quarter high.
    pub dot: Size,
}

/// The landing in a VIEW whose cells are CELL logical pixels, or `None`
/// when it would be too small to draw.
pub(crate) fn geometry(view: romacs_logo::View, cell: Size) -> Option<Geometry> {
    if !view.valid() || cell.width < 1.0 || cell.height < 1.0 {
        return None;
    }
    let placed = romacs_logo::placement(view);
    let px = |(x, y, w, h): (f64, f64, f64, f64)| {
        Rect::new(x, y, x + w, y + h).scale_from_origin(cell.height)
    };
    let (mark, word) = (px(placed.mark), px(placed.word));
    if mark.height() < MIN_MARK {
        return None;
    }
    let margin = (mark.height() * MARGIN).round();
    let block = Rect::new(0.0, 0.0, view.cols as f64 * cell.width, view.rows as f64 * cell.height);
    let shader = mark.inflate(margin, margin);
    let placement = shader.union(word.inflate(2.0, RISE + 2.0)).intersect(block).expand();
    if placement.is_zero_area() {
        return None;
    }
    let local = |r: Rect| r - placement.origin().to_vec2();
    Some(Geometry {
        placement,
        mark: local(shader),
        margin,
        word: local(word),
        dot: Size::new(cell.width / 2.0, cell.height / 4.0),
    })
}

/// The scene drawing GEOMETRY, from its first frame: the nodes at their
/// starting values and the motions toward their targets; unless POLICY is
/// `On`, the targets alone, the still frame.
pub(crate) fn scene(g: &Geometry, policy: Policy) -> Diff {
    let at = |spec: NodeSpec, r: Rect| {
        spec.with(|c| {
            c.transform.x = r.x0 as f32;
            c.transform.y = r.y0 as f32;
            c.width = Dim::Px(r.width() as f32);
            c.height = Dim::Px(r.height() as f32);
        })
    };
    let mut uniforms = vec![0.0; UNIFORMS];
    uniforms[0] = YAW.0;
    uniforms[1] = PITCH.0;
    uniforms[4] = g.dot.width as f32;
    uniforms[5] = g.dot.height as f32;
    uniforms[6] = g.margin as f32;
    let mark = at(
        NodeSpec::new(MARK.0, Kind::Shader(ShaderNode {
            source: mark_wgsl().into(),
            uniforms,
            fps: 0,
            channels: Vec::new(),
        })),
        g.mark,
    );
    let word = at(
        NodeSpec::new(WORDMARK.0, Kind::Path(PathNode {
            data: Arc::new(wordmark(g.word.size())),
            fill: colour(romacs_logo::WORDMARK_COLOUR),
            rule: FillRule::EvenOdd,
            ..PathNode::default()
        })),
        g.word + kurbo::Vec2::new(0.0, RISE),
    )
    .with(|c| c.opacity = 0.0);
    let insert = |index, node| Op::Insert { parent: NodeId::ROOT, index, node };
    let set = |id, prop, value: f32, spec, repeat| Op::Set {
        id,
        prop,
        value: Some(Value::Num(value)),
        animate: (policy == Policy::On).then_some(Animate::Motion { spec, repeat }),
    };
    let forever = |alternate| Repeat::Forever { alternate };
    let reveal = tween(2400, Some(UnitBezier::new(0.75, 0.0, 0.25, 1.0)), TransitionEasing::Linear);
    Diff {
        base: Rev(0),
        ops: vec![
            insert(0, mark),
            insert(1, word),
            set(MARK, Prop::Uniform(0), YAW.1, tween(9000, None, TransitionEasing::EaseInOutCubic), forever(true)),
            set(MARK, Prop::Uniform(1), PITCH.1, tween(13000, None, TransitionEasing::EaseInOutCubic), forever(true)),
            set(MARK, Prop::Uniform(2), 1.0, tween(7000, None, TransitionEasing::Linear), forever(false)),
            set(MARK, Prop::Uniform(3), 1.0, tween(3000, Some(UnitBezier::new(0.25, 0.0, 0.4, 1.0)), TransitionEasing::Linear), Repeat::Once),
            set(WORDMARK, Prop::Opacity, 1.0, reveal, Repeat::Once),
            set(WORDMARK, Prop::Y, g.word.y0 as f32, reveal, Repeat::Once),
        ],
    }
}

/// The diff that moves the scene SCENE_REV built for other geometry onto
/// GEOMETRY, motions running on; a running rise of the wordmark ends where
/// it is.
pub(crate) fn refit(g: &Geometry, scene_rev: Rev) -> Diff {
    let set = |id, prop, value| Op::Set { id, prop, value: Some(value), animate: None };
    let px = |v: f64| Value::Dim(Dim::Px(v as f32));
    let num = |v: f64| Value::Num(v as f32);
    Diff {
        base: scene_rev,
        ops: vec![
            set(MARK, Prop::X, num(g.mark.x0)),
            set(MARK, Prop::Y, num(g.mark.y0)),
            set(MARK, Prop::Width, px(g.mark.width())),
            set(MARK, Prop::Height, px(g.mark.height())),
            set(MARK, Prop::Uniform(4), num(g.dot.width)),
            set(MARK, Prop::Uniform(5), num(g.dot.height)),
            set(MARK, Prop::Uniform(6), num(g.margin)),
            set(WORDMARK, Prop::PathData, Value::Path(Arc::new(wordmark(g.word.size())))),
            set(WORDMARK, Prop::X, num(g.word.x0)),
            set(WORDMARK, Prop::Y, num(g.word.y0)),
            set(WORDMARK, Prop::Width, px(g.word.width())),
            set(WORDMARK, Prop::Height, px(g.word.height())),
        ],
    }
}

/// The distance field's surface size in logical pixels.
pub(crate) fn field_size() -> (u32, u32) {
    let (half, _) = halves();
    (((FIELD_HEIGHT * half.0 / half.1).round()) as u32, FIELD_HEIGHT as u32)
}

fn tween(ms: u64, bezier: Option<UnitBezier>, easing: TransitionEasing) -> MotionSpec {
    MotionDuration::new(Duration::from_millis(ms)).map_or(MotionSpec::Instant, |duration| {
        MotionSpec::Tween(TweenSpec { duration, easing, bezier })
    })
}

fn colour(rgb: u32) -> Color {
    Color::rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

/// The wordmark's outlines scaled into a box of SIZE, from its top left.
fn wordmark(size: Size) -> BezPath {
    let mut path = BezPath::new();
    for d in romacs_logo::wordmark_paths() {
        if let Ok(p) = BezPath::from_svg(d) {
            path.extend(p);
        }
    }
    let bounds = path.bounding_box();
    if bounds.height() <= 0.0 {
        return BezPath::new();
    }
    let scale = size.height / bounds.height();
    path.apply_affine(Affine::scale(scale) * Affine::translate(-bounds.origin().to_vec2()));
    path
}

/// The mark's outlines in object space: its height spans y -1 to 1, y up,
/// centred.  The R (with its counter) first, then the crescent.
fn mark_outlines() -> (BezPath, BezPath) {
    let mut paths = romacs_logo::mark_paths()
        .into_iter()
        .map(|(_, d)| BezPath::from_svg(d).unwrap_or_default());
    let (mut r, mut crescent) = (paths.next().unwrap_or_default(), paths.next().unwrap_or_default());
    let bounds = r.bounding_box().union(crescent.bounding_box());
    let to_object = Affine::scale_non_uniform(2.0 / bounds.height(), -2.0 / bounds.height())
        * Affine::translate(-bounds.center().to_vec2());
    r.apply_affine(to_object);
    crescent.apply_affine(to_object);
    (r, crescent)
}

/// The mark's half extent in object space and the field's.
fn halves() -> ((f64, f64), (f64, f64)) {
    static HALVES: OnceLock<((f64, f64), (f64, f64))> = OnceLock::new();
    *HALVES.get_or_init(|| {
        let (r, crescent) = mark_outlines();
        let b = r.bounding_box().union(crescent.bounding_box());
        let mark = (b.width() / 2.0, b.height() / 2.0);
        (mark, (mark.0 + 0.38, mark.1 + 0.38))
    })
}

/// Line segments along PATH, within TOLERANCE.
fn segments(path: &BezPath, tolerance: f64) -> Vec<[f64; 4]> {
    let mut out = Vec::new();
    let (mut start, mut at) = (kurbo::Point::ZERO, kurbo::Point::ZERO);
    kurbo::flatten(path, tolerance, |el| match el {
        PathEl::MoveTo(p) => (start, at) = (p, p),
        PathEl::LineTo(p) => {
            out.push([at.x, at.y, p.x, p.y]);
            at = p;
        }
        PathEl::ClosePath => {
            if at != start {
                out.push([at.x, at.y, start.x, start.y]);
            }
            at = start;
        }
        _ => {}
    });
    out
}

fn vec2(name: &str, (x, y): (f64, f64)) -> String {
    format!("const {name}: vec2<f32> = vec2<f32>({x:.5}, {y:.5});\n")
}

/// The distance field's WGSL: landing_field.wgsl after the mark's segments.
pub(crate) fn field_wgsl() -> &'static str {
    static SOURCE: OnceLock<String> = OnceLock::new();
    SOURCE.get_or_init(|| {
        let (r, crescent) = mark_outlines();
        let (r, crescent) = (segments(&r, 0.0015), segments(&crescent, 0.0015));
        let all: Vec<String> = r.iter().chain(&crescent)
            .map(|[ax, ay, bx, by]| format!("vec4<f32>({ax:.5}, {ay:.5}, {bx:.5}, {by:.5})"))
            .collect();
        let (_, field) = halves();
        format!(
            "const SEGMENT_COUNT: u32 = {n}u;\nconst R_SEGMENTS: u32 = {nr}u;\n{half}\
             var<private> SEGMENTS: array<vec4<f32>, {n}> = array<vec4<f32>, {n}>(\n{list});\n{body}",
            n = all.len(),
            nr = r.len(),
            half = vec2("FIELD_HALF", field),
            list = all.join(",\n"),
            body = include_str!("landing_field.wgsl"),
        )
    })
}

/// The lit mark's WGSL: landing_mark.wgsl after the extents it shares with
/// the field.
pub(crate) fn mark_wgsl() -> &'static str {
    static SOURCE: OnceLock<String> = OnceLock::new();
    SOURCE.get_or_init(|| {
        let (mark, field) = halves();
        format!("{}{}{}", vec2("FIELD_HALF", field), vec2("MARK_HALF", mark), include_str!("landing_mark.wgsl"))
    })
}

#[cfg(test)]
#[path = "tests/landing_test.rs"]
mod tests;
