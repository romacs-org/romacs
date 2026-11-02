//! One scene: its tree, its running motions and its document clock.
//!
//! [`Scene::apply`] is transactional by construction: it applies every op to
//! a copy (the maps are `Arc`s, copied on first write) and returns the copy
//! only when every op and the final-state checks passed. The caller swaps it
//! in; a rejection leaves the original untouched because nothing wrote to it.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use crate::diff::{Diff, Op, Reason, Reject, Rev};
use crate::motion::{Animate, MediaClock, Policy, Repeat, Track};
use crate::node::{Channel, Kind, Node, NodeId, NodeSpec, SceneId};
use crate::prop::{self, Prop, PropError, Value};

/// At most this many nodes per scene (§2.7).
pub const MAX_NODES: usize = 65_536;
/// At most this deep a tree: every walk over it recurses.
pub const MAX_DEPTH: usize = 256;

#[derive(Clone, Debug, PartialEq)]
pub struct Scene {
    pub id: SceneId,
    pub generation: u32,
    rev: Rev,
    nodes: Arc<BTreeMap<NodeId, Node>>,
    tracks: Arc<BTreeMap<(NodeId, Prop), Track>>,
    clock: MediaClock,
}

impl Scene {
    /// An empty scene: a root group with no children, at revision 0.
    #[must_use]
    pub fn new(id: SceneId, generation: u32) -> Self {
        let root = Node {
            kind: Kind::group(),
            common: crate::node::Common::default(),
            parent: None,
            children: Vec::new(),
            revision: 0,
        };
        Self {
            id,
            generation,
            rev: Rev(0),
            nodes: Arc::new(BTreeMap::from([(NodeId::ROOT, root)])),
            tracks: Arc::default(),
            clock: MediaClock::default(),
        }
    }

    #[must_use]
    pub const fn rev(&self) -> Rev {
        self.rev
    }

    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    #[must_use]
    pub fn nodes(&self) -> &BTreeMap<NodeId, Node> {
        &self.nodes
    }

    #[must_use]
    pub const fn now(&self) -> Duration {
        self.clock.now()
    }

    /// The value presented now: a running track's sample, else the property.
    #[must_use]
    pub fn value(&self, id: NodeId, prop: Prop) -> Option<Value> {
        match self.tracks.get(&(id, prop)) {
            Some(track) => Some(track.sample(self.now())),
            None => prop::get(self.nodes.get(&id)?, prop),
        }
    }

    /// The running motions of one node, for lowering and hit testing.
    pub fn tracks_of(&self, id: NodeId) -> impl Iterator<Item = (Prop, &Track)> {
        self.tracks
            .range((id, Prop::X)..)
            .take_while(move |((n, _), _)| *n == id)
            .map(|((_, p), t)| (*p, t))
    }

    /// Whether any motion is running: the scene has frame demand.
    #[must_use]
    pub fn animating(&self) -> bool {
        !self.tracks.is_empty()
    }

    /// The scene after a presented frame at host time `wall`: the clock moves
    /// if visible, and settled tracks drop. A settled track ends on the value
    /// the node holds, so settling changes no property and no layout.
    #[must_use]
    pub fn tick(&self, wall: Duration, visible: bool) -> Self {
        let clock = self.clock.tick(wall, visible);
        let now = clock.now();
        let mut next = Self {
            clock,
            ..self.clone()
        };
        if self.tracks.values().any(|t| t.settled(now)) {
            Arc::make_mut(&mut next.tracks).retain(|_, t| !t.settled(now));
        }
        next
    }

    /// Under `reduced` or `nil`, running motion snaps: a tween to its
    /// target, which the node already holds; a drive holds its presented
    /// value.
    #[must_use]
    pub fn retime(&self, policy: Policy) -> Self {
        if policy == Policy::On || self.tracks.is_empty() {
            return self.clone();
        }
        let mut next = self.clone();
        let nodes = Arc::make_mut(&mut next.nodes);
        for ((id, prop), track) in self.tracks.iter().filter(|(_, t)| t.is_drive()) {
            if let Some(node) = nodes.get_mut(id) {
                let _ = prop::set(node, *prop, track.sample(self.now()));
            }
        }
        next.tracks = Arc::default();
        next
    }

    /// Apply a diff, or say why not. `self` is never modified.
    pub fn apply(&self, diff: &Diff, policy: Policy) -> Result<Self, Reject> {
        if diff.base != self.rev {
            return Err(Reject::new(Reason::Stale { current: self.rev }));
        }
        let mut next = self.clone();
        next.rev = Rev(self.rev.0 + 1);
        for (index, op) in diff.ops.iter().enumerate() {
            next.apply_op(op, policy).map_err(|r| r.at(index))?;
        }
        next.check_final()?;
        Ok(next)
    }

    fn apply_op(&mut self, op: &Op, policy: Policy) -> Result<(), Reject> {
        match op {
            Op::Insert {
                parent,
                index,
                node,
            } => self.insert(*parent, *index, node),
            Op::Remove { id } => self.remove(*id),
            Op::Move { id, parent, index } => self.relocate(*id, *parent, *index),
            Op::Set {
                id,
                prop,
                value,
                animate,
            } => self.set(*id, *prop, value.clone(), *animate, policy),
        }
    }

    fn group(&self, id: NodeId) -> Result<&Node, Reject> {
        let node = self
            .nodes
            .get(&id)
            .ok_or(Reject::on(Reason::UnknownNode, id))?;
        node.kind
            .is_group()
            .then_some(node)
            .ok_or(Reject::on(Reason::NotAGroup, id))
    }

    fn insert(&mut self, parent: NodeId, index: u32, spec: &NodeSpec) -> Result<(), Reject> {
        let len = self.group(parent)?.children.len();
        if index as usize > len {
            return Err(Reject::on(Reason::IndexOutOfRange, parent));
        }
        let rev = self.rev.0;
        let mut built = Vec::with_capacity(spec.count());
        flatten(spec, parent, rev, &mut built)?;
        let mut seen = std::collections::BTreeSet::new();
        if let Some((id, _)) = built
            .iter()
            .find(|(id, _)| self.nodes.contains_key(id) || !seen.insert(*id))
        {
            return Err(Reject::on(Reason::DuplicateId, *id));
        }
        let nodes = Arc::make_mut(&mut self.nodes);
        nodes.extend(built);
        attach(nodes, parent, index as usize, spec.id);
        Ok(())
    }

    fn remove(&mut self, id: NodeId) -> Result<(), Reject> {
        if id == NodeId::ROOT {
            return Err(Reject::on(Reason::RootFixed, id));
        }
        let parent = self
            .nodes
            .get(&id)
            .ok_or(Reject::on(Reason::UnknownNode, id))?
            .parent;
        let doomed = subtree(&self.nodes, id);
        let nodes = Arc::make_mut(&mut self.nodes);
        if let Some(parent) = parent.and_then(|p| nodes.get_mut(&p)) {
            parent.children.retain(|c| *c != id);
        }
        for n in &doomed {
            nodes.remove(n);
        }
        Arc::make_mut(&mut self.tracks).retain(|(n, _), _| !doomed.contains(n));
        Ok(())
    }

    fn relocate(&mut self, id: NodeId, parent: NodeId, index: u32) -> Result<(), Reject> {
        if id == NodeId::ROOT {
            return Err(Reject::on(Reason::RootFixed, id));
        }
        let old = self
            .nodes
            .get(&id)
            .ok_or(Reject::on(Reason::UnknownNode, id))?
            .parent;
        self.group(parent)?;
        if ancestors(&self.nodes, parent).any(|a| a == id) {
            return Err(Reject::on(Reason::MoveIntoSelf, id));
        }
        let nodes = Arc::make_mut(&mut self.nodes);
        if let Some(old) = old.and_then(|p| nodes.get_mut(&p)) {
            old.children.retain(|c| *c != id);
        }
        if index as usize > nodes[&parent].children.len() {
            return Err(Reject::on(Reason::IndexOutOfRange, parent));
        }
        attach(nodes, parent, index as usize, id);
        Ok(())
    }

    fn set(
        &mut self,
        id: NodeId,
        p: Prop,
        value: Option<Value>,
        animate: Option<Animate>,
        policy: Policy,
    ) -> Result<(), Reject> {
        let reject = |reason| Reject::on(reason, id);
        let from = self.value(id, p).ok_or_else(|| {
            reject(if self.nodes.contains_key(&id) {
                Reason::PropNotOnKind(p)
            } else {
                Reason::UnknownNode
            })
        })?;
        let to = match (value, animate) {
            (Some(v), _) => v,
            (None, Some(Animate::Drive { .. })) => from.clone(),
            (None, _) => return Err(reject(Reason::MissingValue(p))),
        };
        match animate {
            Some(Animate::Motion { spec, repeat }) => {
                if repeat != Repeat::Once
                    && !matches!(
                        spec,
                        romacs_display_protocol::motion_spec::MotionSpec::Tween(_)
                    )
                {
                    return Err(reject(Reason::RepeatNeedsTween(p)));
                }
                if !spec.is_instant() && !Value::interpolates(p, &from, &to) {
                    return Err(reject(Reason::NotAnimatable(p)));
                }
            }
            Some(Animate::Drive { rate })
                if !rate.is_finite() || !Value::interpolates(p, &Value::Num(0.0), &to) =>
            {
                return Err(reject(Reason::NotAnimatable(p)));
            }
            Some(Animate::Drive { .. }) | None => {}
        }
        let rev = self.rev.0;
        let node = Arc::make_mut(&mut self.nodes)
            .get_mut(&id)
            .ok_or(reject(Reason::UnknownNode))?;
        prop::set(node, p, to.clone()).map_err(|e| reject(prop_reason(p, e)))?;
        node.revision = rev;
        let now = self.now();
        let tracks = Arc::make_mut(&mut self.tracks);
        tracks.remove(&(id, p));
        if let Some(track) = animate.and_then(|a| Track::start(p, from, to, a, now, policy)) {
            tracks.insert((id, p), track);
        }
        Ok(())
    }

    /// Checks that hold of the resulting tree, whatever order the ops ran in.
    fn check_final(&self) -> Result<(), Reject> {
        if self.nodes.len() > MAX_NODES {
            return Err(Reject::new(Reason::TooManyNodes { limit: MAX_NODES }));
        }
        if let Some(id) = too_deep(&self.nodes) {
            return Err(Reject::on(Reason::TooDeep { limit: MAX_DEPTH }, id));
        }
        for (id, node) in self.nodes.iter() {
            if let Kind::Shader(s) = &node.kind {
                let local_ok = |n: &NodeId| {
                    self.nodes
                        .get(n)
                        .is_some_and(|t| t.kind.is_target() || *n == NodeId::ROOT)
                };
                if s.channels
                    .iter()
                    .any(|c| matches!(c, Channel::Local(n) if !local_ok(n) || *n == *id))
                {
                    return Err(Reject::on(Reason::BadReference, *id));
                }
            }
            if node.common.anchor.is_some()
                && ancestors(&self.nodes, node.parent.unwrap_or(NodeId::ROOT))
                    .any(|a| self.nodes[&a].kind.flex().is_some())
            {
                return Err(Reject::on(Reason::AnchorInFlex, *id));
            }
        }
        Ok(())
    }
}

/// The reason for a property error.
#[must_use]
pub const fn prop_reason(p: Prop, e: PropError) -> Reason {
    match e {
        PropError::NotOnKind => Reason::PropNotOnKind(p),
        PropError::WrongType => Reason::WrongType(p),
        PropError::OutOfRange => Reason::OutOfRange(p),
    }
}

/// Build the nodes of an inserted subtree, checking each one.
fn flatten(
    spec: &NodeSpec,
    parent: NodeId,
    rev: u64,
    out: &mut Vec<(NodeId, Node)>,
) -> Result<(), Reject> {
    if !spec.children.is_empty() && !spec.kind.is_group() {
        return Err(Reject::on(Reason::NotAGroup, spec.id));
    }
    if spec.id == NodeId::ROOT {
        return Err(Reject::on(Reason::DuplicateId, spec.id));
    }
    let node = Node {
        kind: spec.kind.clone(),
        common: spec.common.clone(),
        parent: Some(parent),
        children: spec.children.iter().map(|c| c.id).collect(),
        revision: rev,
    };
    prop::check_node(&node).map_err(|(p, e)| Reject::on(prop_reason(p, e), spec.id))?;
    out.push((spec.id, node));
    spec.children
        .iter()
        .try_for_each(|c| flatten(c, spec.id, rev, out))
}

fn attach(nodes: &mut BTreeMap<NodeId, Node>, parent: NodeId, index: usize, id: NodeId) {
    if let Some(p) = nodes.get_mut(&parent) {
        p.children.insert(index, id);
    }
    if let Some(n) = nodes.get_mut(&id) {
        n.parent = Some(parent);
    }
}

/// The first node deeper than [`MAX_DEPTH`], breadth first from the root.
fn too_deep(nodes: &BTreeMap<NodeId, Node>) -> Option<NodeId> {
    let mut level = vec![NodeId::ROOT];
    for _ in 0..=MAX_DEPTH {
        level = level
            .iter()
            .flat_map(|n| nodes.get(n).map(|n| n.children.clone()).unwrap_or_default())
            .collect();
        if level.is_empty() {
            return None;
        }
    }
    level.first().copied()
}

/// `id` and every node below it.
#[must_use]
pub fn subtree(nodes: &BTreeMap<NodeId, Node>, id: NodeId) -> Vec<NodeId> {
    let mut out = vec![id];
    let mut i = 0;
    while let Some(n) = out.get(i).copied() {
        out.extend(
            nodes
                .get(&n)
                .map(|n| n.children.as_slice())
                .unwrap_or_default(),
        );
        i += 1;
    }
    out
}

/// `id` and its ancestors up to the root.
pub fn ancestors(nodes: &BTreeMap<NodeId, Node>, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
    std::iter::successors(Some(id), |n| nodes.get(n).and_then(|n| n.parent))
}
