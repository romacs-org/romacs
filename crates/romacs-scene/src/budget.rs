//! The scene pool (§2.7): scenes reserve bytes before anything is allocated.
//!
//! The estimate is deliberately an upper bound computed from data the core
//! has (placement sizes, explicit node sizes, content lengths), so a create or
//! update that would not fit is refused before the renderer allocates.

use std::collections::BTreeMap;

use kurbo::{Rect, Size};

use crate::diff::{Reason, Reject};
use crate::draw::DrawList;
use crate::layout::Solved;
use crate::node::{Dim, Kind, Node, NodeId, SceneId};
use crate::scene::{Scene, ancestors};

/// A resolved texture plus its 4× MSAA colour buffer, RGBA8.
const TARGET_BYTES_PER_PIXEL: f64 = 4.0 * 5.0;
/// Tessellated vertices and indices per path element, generously.
const PATH_BYTES_PER_ELEMENT: u64 = 256;

#[derive(Clone, Debug, PartialEq)]
pub struct Pool {
    capacity: u64,
    reserved: BTreeMap<SceneId, u64>,
}

impl Pool {
    #[must_use]
    pub const fn new(capacity: u64) -> Self {
        Self {
            capacity,
            reserved: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn used(&self) -> u64 {
        self.reserved.values().sum()
    }

    #[must_use]
    pub fn reserved(&self, scene: SceneId) -> u64 {
        self.reserved.get(&scene).copied().unwrap_or(0)
    }

    /// The pool with `scene` holding `bytes` instead of what it held.
    pub fn reserve(&self, scene: SceneId, bytes: u64) -> Result<Self, Reject> {
        let available = self
            .capacity
            .saturating_sub(self.used() - self.reserved(scene));
        if bytes > available {
            return Err(Reject::new(Reason::OverBudget {
                needed: bytes,
                available,
            }));
        }
        let mut next = self.clone();
        next.reserved.insert(scene, bytes);
        Ok(next)
    }

    #[must_use]
    pub fn release(&self, scene: SceneId) -> Self {
        let mut next = self.clone();
        next.reserved.remove(&scene);
        next
    }
}

/// Bytes per target texture of one showing: the root and its target groups.
pub type Targets = BTreeMap<NodeId, u64>;

/// One showing to reserve for: logical size, device scale, and its solved
/// layout for the scene's current revision when there is one.
#[derive(Clone, Copy, Debug)]
pub struct Shown<'a> {
    pub size: Size,
    pub scale: f64,
    pub solved: Option<&'a Solved>,
}

fn pixels(size: Size, scale: f64) -> f64 {
    size.width * size.height * scale * scale * TARGET_BYTES_PER_PIXEL
}

/// The targets of the next frame of a showing: the placement and each
/// target group at its solved size, or its bound before layout.
#[must_use]
pub fn showing(scene: &Scene, s: Shown<'_>) -> Targets {
    let group = |id: NodeId| {
        s.solved
            .and_then(|solved| solved.boxes.get(&id))
            .map_or_else(|| bound(scene, id, s.size), Rect::size)
    };
    scene
        .nodes()
        .iter()
        .filter(|(id, n)| **id != NodeId::ROOT && n.kind.is_target())
        .map(|(id, _)| (*id, pixels(group(*id), s.scale) as u64))
        .chain([(NodeId::ROOT, pixels(s.size, s.scale) as u64)])
        .collect()
}

/// The targets a lowered frame actually allocates, motion included.
#[must_use]
pub fn frame(list: &DrawList, scale: f64) -> Targets {
    list.targets
        .iter()
        .map(|t| (NodeId(t.id.0), pixels(t.size, scale) as u64))
        .collect()
}

/// The scene's own content: paths, text, cells, shader sources.
#[must_use]
pub fn content(scene: &Scene) -> u64 {
    scene.nodes().values().map(content_bytes).sum()
}

/// A target group's size before layout: explicit lengths, fractions of the
/// bound of its parent, and the parent's bound where it is `Auto`.
fn bound(scene: &Scene, id: NodeId, placement: Size) -> Size {
    let chain: Vec<NodeId> = ancestors(scene.nodes(), id).collect();
    chain.iter().rev().skip(1).fold(placement, |parent, n| {
        let c = &scene.nodes()[n].common;
        let dim = |d: Dim, full: f64| match d {
            Dim::Px(v) => f64::from(v),
            Dim::Fraction(f) => f64::from(f) * full,
            Dim::Auto => full,
        };
        Size::new(dim(c.width, parent.width), dim(c.height, parent.height))
    })
}

fn content_bytes(node: &Node) -> u64 {
    let n = |len: usize| len as u64;
    match &node.kind {
        Kind::Path(p) => n(p.data.elements().len()) * PATH_BYTES_PER_ELEMENT,
        Kind::Text(t) => n(t.text.len()),
        Kind::Cells(g) => n(g.glyphs.len()) * 4 + n(g.shades.len()) + n(g.palette.len()) * 16,
        Kind::Shader(s) => n(s.source.len()) + n(s.uniforms.len()) * 4,
        _ => 0,
    }
}
