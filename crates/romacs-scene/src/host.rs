//! The host's scene store (§2.7): every retained scene, its placements, its
//! leases and its share of the pool.
//!
//! This is also the recovery source: it holds each scene's tree, content and
//! motion state outside the renderer, so after a display reset every retained
//! scene is redrawn from here under a new epoch with no VM involvement.
//!
//! Retention is separate from Lisp ownership. A scene stays while Lisp owns
//! it, while a queued or presented frame can still draw it (a presentation
//! lease, which also holds the scenes that frame sampled and its texture's
//! bytes), or while a retained scene samples it (a dependency lease, derived
//! from the dependency graph rather than counted, so it cannot drift).
//! [`Host::collect`] frees what none of these reach.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use kurbo::{Point, Rect, Size};

use crate::budget::{self, Pool, Shown};
use crate::diff::{Diff, Reason, Reject, Rev};
use crate::draw::DrawList;
use crate::graph::{self, Unit};
use crate::layout::{self, Measure, Solved};
use crate::lower::{self, Resolve};
use crate::motion::Policy;
use crate::node::{AnchorId, NodeId, SceneId};
use crate::scene::Scene;

/// One showing of a scene: an inline block, a layer or a page backdrop.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PlacementId(pub u32);

/// What a placement last presented: the snapshot hit testing answers from.
#[derive(Clone, Debug, PartialEq)]
pub struct Presented {
    pub seq: u64,
    pub scene: Scene,
    pub solved: Arc<Solved>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// Logical pixels.
    pub size: Size,
    /// Device pixels per logical pixel.
    pub scale: f64,
    pub anchors: BTreeMap<AnchorId, Rect>,
    solved: Option<(Rev, Arc<Solved>)>,
    pub presented: Option<Presented>,
}

/// A queued or presented frame: what it can still draw from.
#[derive(Clone, Debug, PartialEq)]
struct Lease {
    /// Other scenes the frame sampled.
    sources: BTreeSet<SceneId>,
    /// The bytes of each target its draw list allocates.
    targets: budget::Targets,
}

#[derive(Clone, Debug, PartialEq)]
struct Entry {
    scene: Scene,
    /// Lisp still holds the handle.
    owned: bool,
    placements: BTreeMap<PlacementId, Placement>,
    /// Queued or presented frames, by placement and frame sequence.
    leases: BTreeMap<(PlacementId, u64), Lease>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Host {
    entries: BTreeMap<SceneId, Entry>,
    pool: Pool,
    next_generation: u32,
    epoch: u32,
    policy: Policy,
}

impl Host {
    #[must_use]
    pub fn new(pool_bytes: u64) -> Self {
        Self {
            entries: BTreeMap::new(),
            pool: Pool::new(pool_bytes),
            next_generation: 1,
            epoch: 0,
            policy: Policy::On,
        }
    }

    #[must_use]
    pub fn scene(&self, id: SceneId) -> Option<&Scene> {
        self.entries.get(&id).map(|e| &e.scene)
    }

    /// Scenes retained, owned or not.
    pub fn retained(&self) -> impl Iterator<Item = SceneId> + '_ {
        self.entries.keys().copied()
    }

    #[must_use]
    pub fn is_live(&self, id: SceneId) -> bool {
        self.entries.get(&id).is_some_and(|e| e.owned)
    }

    #[must_use]
    pub const fn pool(&self) -> &Pool {
        &self.pool
    }

    /// Bumped by every display reset; events carry it.
    #[must_use]
    pub const fn epoch(&self) -> u32 {
        self.epoch
    }

    #[must_use]
    pub fn placements(&self, id: SceneId) -> Vec<PlacementId> {
        self.entries
            .get(&id)
            .map(|e| e.placements.keys().copied().collect())
            .unwrap_or_default()
    }

    #[must_use]
    pub fn placement(&self, id: SceneId, placement: PlacementId) -> Option<&Placement> {
        self.entries.get(&id)?.placements.get(&placement)
    }

    /// Create scene `id` from a diff against its empty root (base 0).
    pub fn create(&mut self, id: SceneId, diff: &Diff) -> Result<Rev, Reject> {
        if self.entries.contains_key(&id) {
            return Err(Reject::new(Reason::SceneExists(id)));
        }
        let scene = Scene::new(id, self.next_generation).apply(diff, self.policy)?;
        let entry = Entry {
            scene,
            owned: true,
            placements: BTreeMap::new(),
            leases: BTreeMap::new(),
        };
        let rev = self.commit(id, entry)?;
        self.next_generation += 1;
        Ok(rev)
    }

    /// Apply a diff to a live scene, atomically.
    pub fn update(&mut self, id: SceneId, diff: &Diff) -> Result<Rev, Reject> {
        let entry = self
            .entries
            .get(&id)
            .ok_or(Reject::new(Reason::UnknownScene(id)))?;
        if !entry.owned {
            return Err(Reject::new(Reason::SceneNotLive(id)));
        }
        let scene = entry.scene.apply(diff, self.policy)?;
        self.commit(
            id,
            Entry {
                scene,
                ..entry.clone()
            },
        )
    }

    /// Validate a candidate against every other scene, reserve its bytes, and
    /// only then install it.
    fn commit(&mut self, id: SceneId, entry: Entry) -> Result<Rev, Reject> {
        self.check_refs(&entry.scene)?;
        self.check_acyclic(&entry.scene)?;
        let pool = self.pool.reserve(id, bytes(&entry))?;
        let rev = entry.scene.rev();
        self.pool = pool;
        self.entries.insert(id, entry);
        Ok(rev)
    }

    /// Cross-scene references name a root or target group of the generation
    /// they were made for, both those the candidate makes and those other
    /// scenes make into it. A node this diff touched may name only a live
    /// scene.
    fn check_refs(&self, candidate: &Scene) -> Result<(), Reject> {
        let unit = |n: NodeId| {
            candidate
                .node(n)
                .is_some_and(|node| n == NodeId::ROOT || node.kind.is_target())
        };
        let incoming = self
            .entries
            .values()
            .filter(|e| e.scene.id != candidate.id)
            .flat_map(|e| e.scene.nodes().values())
            .flat_map(|n| graph::scene_refs(&n.kind))
            .filter(|r| r.scene == candidate.id && r.generation == candidate.generation);
        if let Some(r) = incoming.into_iter().find(|r| !unit(r.node)) {
            return Err(Reject::on(Reason::BadReference, r.node));
        }
        for (id, node) in candidate.nodes() {
            for r in graph::scene_refs(&node.kind) {
                let (scene, live) = if r.scene == candidate.id {
                    (Some(candidate), true)
                } else {
                    let e = self.entries.get(&r.scene);
                    (e.map(|e| &e.scene), e.is_some_and(|e| e.owned))
                };
                let names_unit = scene.is_some_and(|s| {
                    s.generation == r.generation
                        && s.node(r.node)
                            .is_some_and(|n| r.node == NodeId::ROOT || n.kind.is_target())
                });
                if !names_unit {
                    return Err(Reject::on(Reason::BadReference, *id));
                }
                if !live && node.revision == candidate.rev().0 {
                    return Err(Reject::on(Reason::SceneNotLive(r.scene), *id));
                }
            }
        }
        Ok(())
    }

    fn check_acyclic(&self, candidate: &Scene) -> Result<(), Reject> {
        let scenes = || {
            self.entries
                .values()
                .map(|e| &e.scene)
                .filter(|s| s.id != candidate.id)
                .chain([candidate])
        };
        let units: BTreeSet<Unit> = scenes().flat_map(graph::units).collect();
        let edges: Vec<_> = scenes().flat_map(graph::edges).collect();
        graph::order(&units, &edges)
            .map(|_| ())
            .map_err(|cycle| Reject::new(Reason::Cycle(cycle)))
    }

    /// Every retained unit, sources first: the order the host renders targets.
    #[must_use]
    pub fn render_order(&self) -> Vec<Unit> {
        let units: BTreeSet<Unit> = self
            .entries
            .values()
            .flat_map(|e| graph::units(&e.scene))
            .collect();
        let edges: Vec<_> = self
            .entries
            .values()
            .flat_map(|e| graph::edges(&e.scene))
            .collect();
        graph::order(&units, &edges).unwrap_or_default()
    }

    /// Lisp let go (`romacs-scene-destroy` or the finalizer). Placements end;
    /// the scene survives its leases. Returns the scenes freed.
    pub fn destroy(&mut self, id: SceneId) -> Vec<SceneId> {
        if let Some(e) = self.entries.get_mut(&id) {
            e.owned = false;
            e.placements.clear();
        }
        self.shrink(id);
        self.collect()
    }

    /// Re-reserve after something was released; a smaller reservation
    /// always fits.
    fn shrink(&mut self, id: SceneId) {
        if let Some(pool) = self
            .entries
            .get(&id)
            .and_then(|e| self.pool.reserve(id, bytes(e)).ok())
        {
            self.pool = pool;
        }
    }

    /// Show the scene at `size` × `scale`, or resize that showing.
    pub fn place(
        &mut self,
        id: SceneId,
        placement: PlacementId,
        size: Size,
        scale: f64,
    ) -> Result<(), Reject> {
        let entry = self
            .entries
            .get(&id)
            .ok_or(Reject::new(Reason::UnknownScene(id)))?;
        if !entry.owned {
            return Err(Reject::new(Reason::SceneNotLive(id)));
        }
        let mut next = entry.clone();
        let old = next.placements.remove(&placement);
        let anchors = old.map(|p| p.anchors).unwrap_or_default();
        next.placements.insert(
            placement,
            Placement {
                size,
                scale,
                anchors,
                solved: None,
                presented: None,
            },
        );
        self.pool = self.pool.reserve(id, bytes(&next))?;
        self.entries.insert(id, next);
        Ok(())
    }

    /// Stop showing; its queued frames keep their bytes until retired.
    pub fn unplace(&mut self, id: SceneId, placement: PlacementId) {
        if let Some(e) = self.entries.get_mut(&id) {
            e.placements.remove(&placement);
        }
        self.shrink(id);
    }

    /// The anchor boxes `layout2` published for the window of a placement.
    pub fn set_anchors(
        &mut self,
        id: SceneId,
        placement: PlacementId,
        anchors: BTreeMap<AnchorId, Rect>,
    ) {
        if let Some(p) = self
            .entries
            .get_mut(&id)
            .and_then(|e| e.placements.get_mut(&placement))
            && p.anchors != anchors
        {
            p.anchors = anchors;
            p.solved = None;
        }
    }

    /// Font state changed (face, `text-scale`, default font, DPI): every
    /// text measurement is stale.
    pub fn fonts_changed(&mut self) {
        for p in self
            .entries
            .values_mut()
            .flat_map(|e| e.placements.values_mut())
        {
            p.solved = None;
        }
    }

    /// The solved layout of a placement, cached per revision.
    pub fn layout(
        &mut self,
        id: SceneId,
        placement: PlacementId,
        measure: &mut dyn Measure,
    ) -> Option<Arc<Solved>> {
        let e = self.entries.get_mut(&id)?;
        let p = e.placements.get_mut(&placement)?;
        let rev = e.scene.rev();
        match &p.solved {
            Some((r, s)) if *r == rev => Some(s.clone()),
            _ => {
                let solved = Arc::new(layout::solve(&e.scene, p.size, &p.anchors, measure));
                p.solved = Some((rev, solved.clone()));
                Some(solved)
            }
        }
    }

    /// Lower a placement for frame `seq`, take a presentation lease on it,
    /// and remember it as what hit testing answers from. The targets its
    /// draw list allocates (at their motion-sampled sizes) are reserved
    /// first; a frame that does not fit is refused with the shortfall.
    pub fn present(
        &mut self,
        id: SceneId,
        placement: PlacementId,
        seq: u64,
        measure: &mut dyn Measure,
        resolve: &mut dyn Resolve,
    ) -> Result<DrawList, Reject> {
        let solved = self
            .layout(id, placement, measure)
            .ok_or(Reject::new(Reason::UnknownScene(id)))?;
        let e = &self.entries[&id];
        let scale = e.placements[&placement].scale;
        let list = lower::lower(&e.scene, &solved, resolve);
        let lease = Lease {
            sources: graph::edges(&e.scene)
                .into_iter()
                .map(|edge| edge.source.0)
                .filter(|s| *s != id)
                .collect(),
            targets: budget::frame(&list, scale),
        };
        let mut next = e.clone();
        next.leases.insert((placement, seq), lease);
        self.pool = self.pool.reserve(id, bytes(&next))?;
        if let Some(p) = next.placements.get_mut(&placement) {
            p.presented = Some(Presented {
                seq,
                scene: next.scene.clone(),
                solved,
            });
        }
        self.entries.insert(id, next);
        Ok(list)
    }

    /// Frame `seq` of a placement can no longer draw: drop its lease.
    pub fn retire(&mut self, id: SceneId, placement: PlacementId, seq: u64) -> Vec<SceneId> {
        if let Some(e) = self.entries.get_mut(&id) {
            e.leases.remove(&(placement, seq));
        }
        self.shrink(id);
        self.collect()
    }

    /// The node under `point` in what the placement last presented.
    #[must_use]
    pub fn hit(
        &self,
        id: SceneId,
        placement: PlacementId,
        point: Point,
    ) -> Option<(NodeId, Point)> {
        let presented = self.placement(id, placement)?.presented.as_ref()?;
        crate::hit::hit(&presented.scene, &presented.solved, point)
    }

    /// Advance every scene's clock for a presented frame at host time `wall`.
    pub fn tick(&mut self, wall: Duration, visible: &dyn Fn(SceneId) -> bool) {
        for (id, e) in &mut self.entries {
            e.scene = e.scene.tick(wall, visible(*id));
        }
    }

    /// Scenes with a running motion: they have frame demand while visible.
    #[must_use]
    pub fn animating(&self) -> Vec<SceneId> {
        self.entries
            .iter()
            .filter(|(_, e)| e.scene.animating())
            .map(|(id, _)| *id)
            .collect()
    }

    /// `romacs-animation` changed: motions it no longer allows settle now.
    pub fn set_policy(&mut self, policy: Policy) {
        self.policy = policy;
        for e in self.entries.values_mut() {
            e.scene = e.scene.retime(policy);
        }
    }

    /// The display was reset: every frame and GPU object is gone. Frames'
    /// leases drop, layouts are re-solved, and every retained scene is
    /// redrawn from this store under a new epoch. Returns the scenes kept.
    pub fn reset(&mut self) -> Vec<SceneId> {
        self.epoch += 1;
        for e in self.entries.values_mut() {
            e.leases.clear();
            for p in e.placements.values_mut() {
                p.solved = None;
                p.presented = None;
            }
        }
        self.collect();
        let kept: Vec<SceneId> = self.retained().collect();
        kept.iter().for_each(|id| self.shrink(*id));
        kept
    }

    /// Free every scene nothing reaches: the roots are owned scenes and
    /// scenes with a frame lease, and a retained scene reaches the scenes it
    /// samples now and those its leased frames sampled.
    pub fn collect(&mut self) -> Vec<SceneId> {
        let mut reached: BTreeSet<SceneId> = self
            .entries
            .iter()
            .filter(|(_, e)| e.owned || !e.leases.is_empty())
            .map(|(id, _)| *id)
            .collect();
        let mut pending: Vec<SceneId> = reached.iter().copied().collect();
        while let Some(id) = pending.pop() {
            let Some(e) = self.entries.get(&id) else {
                continue;
            };
            let sampled = graph::edges(&e.scene).into_iter().map(|edge| edge.source.0);
            let leased = e.leases.values().flat_map(|l| l.sources.iter().copied());
            for source in sampled.chain(leased).collect::<Vec<_>>() {
                if reached.insert(source) {
                    pending.push(source);
                }
            }
        }
        let freed: Vec<SceneId> = self
            .entries
            .keys()
            .filter(|id| !reached.contains(id))
            .copied()
            .collect();
        for id in &freed {
            self.entries.remove(id);
            self.pool = self.pool.release(*id);
        }
        freed
    }
}

/// A scene's bytes: its content, and per showing and target texture the
/// larger of what its next frame needs and what its leased frames still
/// hold. A frame stays charged at what it allocated until it retires,
/// whatever the scene or the placement became since; a showing's frames
/// reuse one texture per target, so a target is charged once at its largest.
fn bytes(entry: &Entry) -> u64 {
    let rev = entry.scene.rev();
    let next = entry.placements.iter().map(|(id, p)| {
        let solved = p
            .solved
            .as_ref()
            .filter(|(r, _)| *r == rev)
            .map(|(_, s)| s.as_ref());
        let shown = Shown {
            size: p.size,
            scale: p.scale,
            solved,
        };
        (*id, budget::showing(&entry.scene, shown))
    });
    let leased = entry
        .leases
        .iter()
        .map(|((id, _), l)| (*id, l.targets.clone()));
    let mut charged: BTreeMap<(PlacementId, NodeId), u64> = BTreeMap::new();
    for (placement, targets) in next.chain(leased) {
        for (target, b) in targets {
            let c = charged.entry((placement, target)).or_default();
            *c = (*c).max(b);
        }
    }
    budget::content(&entry.scene) + charged.values().sum::<u64>()
}
