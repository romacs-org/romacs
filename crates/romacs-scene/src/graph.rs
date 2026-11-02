//! The dependency graph of rendered targets (§2.2, §2.4).
//!
//! A unit is a texture the host renders: a scene's root or one of its target
//! groups. A unit depends on every unit something inside it samples (shader
//! channels, an `image` of another scene) and on every target group nested
//! directly in it, whose texture it composites. Sources render first.

use std::collections::{BTreeMap, BTreeSet};

use crate::node::{Channel, ImageSource, Kind, NodeId, SceneId, SceneRef};
use crate::scene::{Scene, ancestors};

/// A rendered texture: `(scene, root or target group)`.
pub type Unit = (SceneId, NodeId);

/// `consumer` samples or composites `source`.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Edge {
    pub consumer: Unit,
    pub source: Unit,
}

/// The unit a node draws into: its nearest target-group ancestor, or the root.
#[must_use]
pub fn enclosing_target(scene: &Scene, id: NodeId) -> NodeId {
    let start = scene
        .node(id)
        .and_then(|n| n.parent)
        .unwrap_or(NodeId::ROOT);
    ancestors(scene.nodes(), start)
        .find(|a| *a == NodeId::ROOT || scene.node(*a).is_some_and(|n| n.kind.is_target()))
        .unwrap_or(NodeId::ROOT)
}

/// Cross-scene references made by a node.
pub fn scene_refs(kind: &Kind) -> impl Iterator<Item = SceneRef> + '_ {
    let channels = match kind {
        Kind::Shader(s) => s.channels.as_slice(),
        _ => &[],
    };
    let image = match kind {
        Kind::Image(i) => match i.source {
            ImageSource::Scene(r) => Some(r),
            ImageSource::File(_) => None,
        },
        _ => None,
    };
    channels
        .iter()
        .filter_map(|c| match c {
            Channel::Scene(r) => Some(*r),
            Channel::Local(_) => None,
        })
        .chain(image)
}

/// Every edge whose consumer lies in `scene`.
#[must_use]
pub fn edges(scene: &Scene) -> Vec<Edge> {
    let unit = |n| (scene.id, n);
    scene
        .nodes()
        .iter()
        .flat_map(|(id, node)| {
            let consumer = unit(enclosing_target(scene, *id));
            let nested = (node.kind.is_target() && *id != NodeId::ROOT).then_some(unit(*id));
            let locals = match &node.kind {
                Kind::Shader(s) => s
                    .channels
                    .iter()
                    .filter_map(|c| match c {
                        Channel::Local(n) => Some(unit(*n)),
                        Channel::Scene(_) => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            let remote = scene_refs(&node.kind).map(|r| (r.scene, r.node));
            nested
                .into_iter()
                .chain(locals)
                .chain(remote)
                .map(move |source| Edge { consumer, source })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// A topological order of `units` under `edges` (sources first), or a cycle.
///
/// Deterministic: among ready units the smallest goes first.
pub fn order(units: &BTreeSet<Unit>, edges: &[Edge]) -> Result<Vec<Unit>, Vec<Unit>> {
    let mut pending: BTreeMap<Unit, usize> = units.iter().map(|u| (*u, 0)).collect();
    let mut consumers: BTreeMap<Unit, Vec<Unit>> = BTreeMap::new();
    for e in edges
        .iter()
        .filter(|e| units.contains(&e.consumer) && units.contains(&e.source))
    {
        *pending.entry(e.consumer).or_default() += 1;
        consumers.entry(e.source).or_default().push(e.consumer);
    }
    let mut ready: BTreeSet<Unit> = pending
        .iter()
        .filter(|(_, n)| **n == 0)
        .map(|(u, _)| *u)
        .collect();
    let mut out = Vec::with_capacity(units.len());
    while let Some(u) = ready.pop_first() {
        out.push(u);
        for c in consumers.get(&u).map(Vec::as_slice).unwrap_or_default() {
            let n = pending.get_mut(c).expect("consumer is a unit");
            *n -= 1;
            if *n == 0 {
                ready.insert(*c);
            }
        }
    }
    if out.len() == units.len() {
        return Ok(out);
    }
    let done: BTreeSet<Unit> = out.into_iter().collect();
    Err(find_cycle(
        units
            .iter()
            .filter(|u| !done.contains(u))
            .copied()
            .collect(),
        edges,
    ))
}

/// A cycle among `stuck`, every one of which lies on or leads to a cycle.
fn find_cycle(stuck: BTreeSet<Unit>, edges: &[Edge]) -> Vec<Unit> {
    let next = |u: Unit| {
        edges
            .iter()
            .filter(|e| e.consumer == u && stuck.contains(&e.source))
            .map(|e| e.source)
            .min()
    };
    let mut path = Vec::new();
    let mut at = stuck.first().copied();
    while let Some(u) = at {
        if let Some(i) = path.iter().position(|p| *p == u) {
            return path.split_off(i);
        }
        path.push(u);
        at = next(u);
    }
    path
}

/// The units of one scene: its root and its target groups.
#[must_use]
pub fn units(scene: &Scene) -> BTreeSet<Unit> {
    scene
        .nodes()
        .iter()
        .filter(|(id, n)| **id == NodeId::ROOT || n.kind.is_target())
        .map(|(id, _)| (scene.id, *id))
        .collect()
}
