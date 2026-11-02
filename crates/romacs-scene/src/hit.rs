//! Hit testing against what a placement presented (§2.2).
//!
//! The query runs the composition walk over the presented snapshot (the
//! scene as of the presented revision and document time), so a click during
//! a glide hits the node where the user saw it, not where it is heading.

use std::collections::BTreeMap;

use kurbo::{Point, Rect};

use crate::layout::Solved;
use crate::node::NodeId;
use crate::scene::{Scene, ancestors};
use crate::walk::walk;

/// The topmost drawn leaf under `point` (placement pixels), with the point
/// in that node's local pixels.
///
/// A node inside a target group shows only within that group's texture, so
/// the point must also fall in every enclosing target's box, tested in the
/// target's own coordinates (a rotated target is not its bounding box).
#[must_use]
pub fn hit(scene: &Scene, solved: &Solved, point: Point) -> Option<(NodeId, Point)> {
    let mut targets: BTreeMap<NodeId, bool> = BTreeMap::new();
    let mut found = None;
    walk(scene, solved, &mut |p| {
        let local = (p.world.determinant().abs() >= 1e-12).then(|| p.world.inverse() * point);
        let inside =
            local.is_some_and(|l| contains(Rect::from_origin_size(Point::ZERO, p.size), l));
        // The root's texture is the placement itself, bounded by world_clip.
        if p.node.kind.is_target() && p.id != NodeId::ROOT {
            targets.insert(p.id, inside);
        }
        if p.node.kind.is_group() || p.world_opacity <= 0.0 || !inside {
            return;
        }
        if p.world_clip.is_some_and(|c| !contains(c, point)) {
            return;
        }
        let in_targets = ancestors(scene.nodes(), p.id)
            .skip(1)
            .all(|a| targets.get(&a).copied().unwrap_or(true));
        if in_targets {
            found = local.map(|l| (p.id, l));
        }
    });
    found
}

/// Half-open containment, so adjacent boxes never both claim an edge.
fn contains(r: Rect, p: Point) -> bool {
    p.x >= r.x0 && p.x < r.x1 && p.y >= r.y0 && p.y < r.y1
}
