//! Lowering one placement to the renderer's [`DrawList`].
//!
//! A fold over [`crate::walk`]: each drawn node becomes zero or more items in
//! its target, and the targets are ordered by the dependency graph with the
//! placement root last, as `draw.rs`'s contract requires.

use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use kurbo::{Rect, Size};

use crate::draw::{
    self, CellsPaint, DrawList, Fill, Item, Paint, PathPaint, PathStroke, RectPaint, SamplePaint,
    Source, Target, TargetId, TextPaint,
};
use crate::graph;
use crate::layout::{Content, Solved};
use crate::node::{Color, Fit, ImageSource, Kind, NodeId, SceneId};
use crate::scene::Scene;
use crate::walk::{Placed, walk};

/// Host resources a scene refers to by node.
pub trait Resolve {
    /// The texture of an `image` node and its natural size: a cached image,
    /// or another scene's target installed as a surface.
    fn image(
        &mut self,
        scene: SceneId,
        node: NodeId,
        source: &ImageSource,
    ) -> Option<(Source, Size)>;
    /// The surface a `shader` or `model` node renders into.
    fn surface(&mut self, scene: SceneId, node: NodeId) -> Option<u32>;
}

/// Resolves nothing: image, shader and model nodes draw nothing.
pub struct NoResources;

impl Resolve for NoResources {
    fn image(&mut self, _: SceneId, _: NodeId, _: &ImageSource) -> Option<(Source, Size)> {
        None
    }

    fn surface(&mut self, _: SceneId, _: NodeId) -> Option<u32> {
        None
    }
}

/// The draw list of `scene` laid out as `solved`, at its document time.
pub fn lower(scene: &Scene, solved: &Solved, resolve: &mut dyn Resolve) -> DrawList {
    let mut items: BTreeMap<NodeId, Vec<Item>> = BTreeMap::new();
    let mut sizes = BTreeMap::from([(NodeId::ROOT, solved.size)]);
    walk(scene, solved, &mut |p| {
        if p.node.kind.is_target() {
            sizes.insert(p.id, p.size);
        }
        if p.id == NodeId::ROOT || p.opacity <= 0.0 {
            return;
        }
        let out = items.entry(p.target).or_default();
        out.extend(
            paints(scene.id, &p, solved, resolve)
                .into_iter()
                .map(|(paint, clip)| Item {
                    node: p.id.0,
                    transform: p.transform,
                    opacity: p.opacity,
                    clip: clip.or(p.clip),
                    paint,
                }),
        );
    });
    let order = graph::order(&graph::units(scene), &graph::edges(scene)).unwrap_or_default();
    let targets = order
        .into_iter()
        .map(|(_, id)| id)
        .filter(|id| *id != NodeId::ROOT && sizes.contains_key(id))
        .chain([NodeId::ROOT])
        .map(|id| Target {
            id: TargetId(id.0),
            size: sizes[&id],
            clear: Color::TRANSPARENT,
            items: items.remove(&id).unwrap_or_default(),
        })
        .collect();
    DrawList { targets }
}

/// A node's paints, each with a clip override when it needs one.
fn paints(
    scene: SceneId,
    p: &Placed<'_>,
    solved: &Solved,
    resolve: &mut dyn Resolve,
) -> Vec<(Paint, Option<Rect>)> {
    let bounds = Rect::from_origin_size((0.0, 0.0), p.size);
    let some = |c: Color| (c.a > 0.0).then_some(c);
    let sample = |source, rect| {
        Paint::Sample(SamplePaint {
            source,
            rect,
            radius: 0.0,
        })
    };
    match &p.node.kind {
        Kind::Group { target: true, .. } => {
            vec![(sample(Source::Target(TargetId(p.id.0)), bounds), None)]
        }
        Kind::Group { .. } => Vec::new(),
        Kind::Rect(r) => vec![(
            Paint::Rect(RectPaint {
                rect: bounds,
                radius: f64::from(r.radius),
                fill: some(r.fill),
                stroke: some(r.stroke.color)
                    .filter(|_| r.stroke.width > 0.0)
                    .map(|color| draw::Stroke {
                        width: f64::from(r.stroke.width),
                        color,
                    }),
                shadow: some(r.shadow.color).map(|color| draw::Shadow {
                    offset: kurbo::Vec2::new(f64::from(r.shadow.dx), f64::from(r.shadow.dy)),
                    blur: f64::from(r.shadow.blur),
                    spread: f64::from(r.shadow.spread),
                    color,
                }),
                glow: some(r.glow.color)
                    .filter(|_| r.glow.radius > 0.0)
                    .map(|color| draw::Glow {
                        radius: f64::from(r.glow.radius),
                        color,
                    }),
            }),
            None,
        )],
        Kind::Path(path) => {
            let fill = some(path.fill).map(|color| Fill {
                color,
                rule: path.rule,
            });
            let stroke = some(path.stroke.color)
                .filter(|_| path.stroke.width > 0.0)
                .map(|color| PathStroke {
                    width: f64::from(path.stroke.width),
                    color,
                    join: path.join,
                    cap: path.cap,
                    dash: path.dash.iter().map(|d| f64::from(*d)).collect(),
                    dash_offset: f64::from(path.dash_offset),
                });
            vec![(
                Paint::Path(PathPaint {
                    path: path.data.clone(),
                    revision: tessellation_revision(
                        p.node.revision,
                        fill.as_ref(),
                        stroke.as_ref(),
                    ),
                    fill,
                    stroke,
                }),
                None,
            )]
        }
        Kind::Text(t) => match solved.content.get(&p.id) {
            Some(Content::Lines(lines)) => lines
                .iter()
                .map(|line| {
                    let paint = Paint::Text(TextPaint {
                        origin: kurbo::Point::new(0.0, line.baseline),
                        text: line.text.clone(),
                        font: t.font.clone(),
                        color: t.color,
                    });
                    (paint, None)
                })
                .collect(),
            _ => Vec::new(),
        },
        Kind::Cells(g) => {
            let Some(Content::Cell(cell)) = solved.content.get(&p.id) else {
                return Vec::new();
            };
            let n = usize::from(g.cols) * usize::from(g.rows);
            let shades = if g.shades.is_empty() {
                vec![0; n].into()
            } else {
                g.shades.clone()
            };
            vec![(
                Paint::Cells(CellsPaint {
                    origin: kurbo::Point::ZERO,
                    cols: g.cols,
                    rows: g.rows,
                    cell: *cell,
                    glyphs: g.glyphs.clone(),
                    shades,
                    palette: g.palette.clone(),
                    font: g.font.clone(),
                }),
                None,
            )]
        }
        Kind::Image(i) => resolve
            .image(scene, p.id, &i.source)
            .map(|(source, natural)| {
                let rect = fit(i.fit, natural, p.size);
                let overflows = !bounds.contains_rect(rect);
                let clip = overflows.then(|| {
                    let own = p.transform.transform_rect_bbox(bounds);
                    p.clip.map_or(own, |c| c.intersect(own))
                });
                (sample(source, rect), clip)
            })
            .into_iter()
            .collect(),
        Kind::Shader(_) | Kind::Model(_) => resolve
            .surface(scene, p.id)
            .map(|id| (sample(Source::Surface(id), bounds), None))
            .into_iter()
            .collect(),
    }
}

/// Where an image of `natural` size lands in a box of `size`.
#[must_use]
pub fn fit(fit: Fit, natural: Size, size: Size) -> Rect {
    if natural.width <= 0.0 || natural.height <= 0.0 {
        return Rect::from_origin_size((0.0, 0.0), size);
    }
    let (sx, sy) = (size.width / natural.width, size.height / natural.height);
    let scaled = match fit {
        Fit::Fill => return Rect::from_origin_size((0.0, 0.0), size),
        Fit::Contain => natural * sx.min(sy),
        Fit::Cover => natural * sx.max(sy),
        Fit::None => natural,
    };
    let origin = (
        (size.width - scaled.width) / 2.0,
        (size.height - scaled.height) / 2.0,
    );
    Rect::from_origin_size(origin, scaled)
}

/// Identifies a path's tessellation inputs: the node's last property change
/// plus the sampled fill and stroke, so a settling stroke-width motion also
/// changes it.
fn tessellation_revision(
    node_revision: u64,
    fill: Option<&Fill>,
    stroke: Option<&PathStroke>,
) -> u64 {
    let mut h = std::hash::DefaultHasher::new();
    node_revision.hash(&mut h);
    if let Some(f) = fill {
        [f.color.r, f.color.g, f.color.b, f.color.a]
            .map(f32::to_bits)
            .hash(&mut h);
        (f.rule as u8).hash(&mut h);
    }
    if let Some(s) = stroke {
        [s.color.r, s.color.g, s.color.b, s.color.a]
            .map(f32::to_bits)
            .hash(&mut h);
        s.width.to_bits().hash(&mut h);
        (s.join as u8, s.cap as u8).hash(&mut h);
        s.dash
            .iter()
            .map(|d| d.to_bits())
            .collect::<Vec<_>>()
            .hash(&mut h);
        s.dash_offset.to_bits().hash(&mut h);
    }
    h.finish()
}
