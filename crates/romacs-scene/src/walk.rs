//! The composition walk: every drawn node of one placement, in paint order,
//! with its transform, opacity and clip composed at the scene's document time.
//!
//! Lowering to a [`crate::draw::DrawList`] and hit testing are both folds
//! over this walk, so a click hits exactly what was drawn.

use std::borrow::Cow;

use kurbo::{Affine, Point, Rect, Size};

use crate::layout::Solved;
use crate::node::{Dim, Node, NodeId};
use crate::prop::{self, Prop};
use crate::scene::Scene;

/// One drawn node.
#[derive(Clone, Debug)]
pub struct Placed<'a> {
    pub id: NodeId,
    /// The node with its running motions sampled in.
    pub node: Cow<'a, Node>,
    /// The texture it draws into: the root or a target group.
    pub target: NodeId,
    /// Node-local to target coordinates.
    pub transform: Affine,
    /// Node-local to placement coordinates.
    pub world: Affine,
    /// Composed opacity within its target.
    pub opacity: f32,
    /// Composed opacity on screen.
    pub world_opacity: f32,
    /// Axis-aligned clip in target coordinates.
    pub clip: Option<Rect>,
    /// Axis-aligned clip in placement coordinates.
    pub world_clip: Option<Rect>,
    /// The drawn box, node-local from the origin.
    pub size: Size,
}

/// Visit every drawn node of `scene` laid out as `solved`, in paint order.
///
/// A target group is visited once as the item that composites its texture
/// (in its parent's target); its subtree follows, drawn into its own target
/// with a fresh transform, opacity and clip.
pub fn walk<'a>(scene: &'a Scene, solved: &Solved, visit: &mut dyn FnMut(Placed<'a>)) {
    let root = Frame {
        target: NodeId::ROOT,
        transform: Affine::IDENTITY,
        world: Affine::IDENTITY,
        opacity: 1.0,
        world_opacity: 1.0,
        clip: None,
        // Nothing outside the placement's texture is on screen.
        world_clip: Some(Rect::from_origin_size(Point::ZERO, solved.size)),
    };
    descend(scene, solved, NodeId::ROOT, root, visit);
}

#[derive(Clone, Copy)]
struct Frame {
    target: NodeId,
    transform: Affine,
    world: Affine,
    opacity: f32,
    world_opacity: f32,
    clip: Option<Rect>,
    world_clip: Option<Rect>,
}

fn descend<'a>(
    scene: &'a Scene,
    solved: &Solved,
    id: NodeId,
    parent: Frame,
    visit: &mut dyn FnMut(Placed<'a>),
) {
    let (Some(node), Some(rect)) = (effective(scene, id), solved.boxes.get(&id)) else {
        return;
    };
    if !node.common.visible {
        return;
    }
    let size = drawn_size(scene, id, &node, rect.size());
    let local = local_transform(&node, rect.origin().to_vec2(), size);
    let opacity = node.common.opacity;
    // The root draws nothing itself, but its transform and opacity carry its
    // whole subtree: a page transition is one motion on the root.
    let (transform, world) = (parent.transform * local, parent.world * local);
    let own_clip = node
        .common
        .clip
        .then(|| Rect::from_origin_size((0.0, 0.0), size));
    let clip = intersect(
        parent.clip,
        own_clip.map(|r| transform.transform_rect_bbox(r)),
    );
    let world_clip = intersect(
        parent.world_clip,
        own_clip.map(|r| world.transform_rect_bbox(r)),
    );
    let placed = Placed {
        id,
        target: parent.target,
        transform,
        world,
        opacity: parent.opacity * opacity,
        world_opacity: parent.world_opacity * opacity,
        // A node's own clip bounds its subtree, not its own paint (a glow
        // reaches past its box).
        clip: parent.clip,
        world_clip: parent.world_clip,
        size,
        node: node.clone(),
    };
    let here = if node.kind.is_target() && id != NodeId::ROOT {
        Frame {
            target: id,
            transform: Affine::IDENTITY,
            world,
            opacity: 1.0,
            world_opacity: placed.world_opacity,
            clip: own_clip,
            world_clip,
        }
    } else {
        Frame {
            target: parent.target,
            transform,
            world,
            opacity: placed.opacity,
            world_opacity: placed.world_opacity,
            clip,
            world_clip,
        }
    };
    let mut children = node.children.clone();
    children.sort_by_key(|c| scene.node(*c).map_or(0, |n| n.common.z));
    visit(placed);
    for c in children {
        descend(scene, solved, c, here, visit);
    }
}

/// The node with every running motion sampled at the scene's document time.
fn effective(scene: &Scene, id: NodeId) -> Option<Cow<'_, Node>> {
    let node = scene.node(id)?;
    let mut tracks = scene.tracks_of(id).peekable();
    if tracks.peek().is_none() {
        return Some(Cow::Borrowed(node));
    }
    let mut node = node.clone();
    for (p, track) in tracks {
        let _ = prop::set(&mut node, p, track.sample(scene.now()));
    }
    Some(Cow::Owned(node))
}

/// The laid-out size, except where a size motion is running: that moves the
/// drawn box only, from the sampled width or height.
fn drawn_size(scene: &Scene, id: NodeId, node: &Node, laid_out: Size) -> Size {
    let moving = |p: Prop| scene.tracks_of(id).any(|(q, _)| q == p);
    let px = |p: Prop, d: Dim, fallback: f64| match (moving(p), d) {
        (true, Dim::Px(v)) => f64::from(v),
        _ => fallback,
    };
    Size::new(
        px(Prop::Width, node.common.width, laid_out.width),
        px(Prop::Height, node.common.height, laid_out.height),
    )
}

/// Box offset, translation, then rotation and scale about the origin point.
fn local_transform(node: &Node, offset: kurbo::Vec2, size: Size) -> Affine {
    let t = &node.common.transform;
    let origin = kurbo::Vec2::new(
        f64::from(t.origin_x) * size.width,
        f64::from(t.origin_y) * size.height,
    );
    Affine::translate(offset + kurbo::Vec2::new(f64::from(t.x), f64::from(t.y)))
        * Affine::translate(origin)
        * Affine::rotate(f64::from(t.rotate).to_radians())
        * Affine::scale_non_uniform(f64::from(t.scale_x), f64::from(t.scale_y))
        * Affine::translate(-origin)
}

fn intersect(a: Option<Rect>, b: Option<Rect>) -> Option<Rect> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.intersect(b)),
        (a, b) => a.or(b),
    }
}
