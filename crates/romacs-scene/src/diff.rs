//! Scene diffs and their rejections (§2.2).
//!
//! A diff is `(:base REV OPS...)`. It applies as a whole or not at all, so a
//! rejection names the first offending op and node and nothing else changed.

use std::fmt;

use crate::motion::Animate;
use crate::node::{NodeId, NodeSpec, SceneId};
use crate::prop::{Prop, Value};

/// A scene revision. A fresh scene is at 0; every applied diff bumps it.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Rev(pub u64);

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Insert a subtree as child `index` of `parent`.
    Insert {
        parent: NodeId,
        index: u32,
        node: NodeSpec,
    },
    /// Remove a node and its subtree.
    Remove { id: NodeId },
    /// Re-parent a node (and its subtree) as child `index` of `parent`.
    Move {
        id: NodeId,
        parent: NodeId,
        index: u32,
    },
    /// Set a property. `value` may be omitted only for a drive, which then
    /// starts from the value presented now.
    Set {
        id: NodeId,
        prop: Prop,
        value: Option<Value>,
        animate: Option<Animate>,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Diff {
    pub base: Rev,
    pub ops: Vec<Op>,
}

/// Why a diff was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum Reason {
    /// `base` is not the scene's revision; rebase on `current`.
    Stale {
        current: Rev,
    },
    UnknownScene(SceneId),
    SceneExists(SceneId),
    /// The scene was destroyed and survives only for its leases.
    SceneNotLive(SceneId),
    UnknownNode,
    DuplicateId,
    NotAGroup,
    IndexOutOfRange,
    /// The root can be changed but not removed or moved.
    RootFixed,
    /// A move would place a node inside its own subtree.
    MoveIntoSelf,
    PropNotOnKind(Prop),
    WrongType(Prop),
    OutOfRange(Prop),
    /// The property or its values cannot be interpolated.
    NotAnimatable(Prop),
    /// `:repeat` needs a tween, whose cycle has a length.
    RepeatNeedsTween(Prop),
    /// A drive needs a value, and only a set with a drive may omit one.
    MissingValue(Prop),
    /// A channel or scene image names something that is not a scene root or
    /// a target group (§2.4), or a stale generation.
    BadReference,
    /// The dependency graph would have a cycle through these targets.
    Cycle(Vec<(SceneId, NodeId)>),
    /// An anchored node sits inside a flex group; anchors place a node in
    /// scene coordinates, which flex layout would contradict.
    AnchorInFlex,
    TooManyNodes {
        limit: usize,
    },
    TooDeep {
        limit: usize,
    },
    OverBudget {
        needed: u64,
        available: u64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reject {
    pub reason: Reason,
    /// Index of the offending op, when one op is to blame.
    pub op: Option<usize>,
    pub node: Option<NodeId>,
}

impl Reject {
    #[must_use]
    pub const fn new(reason: Reason) -> Self {
        Self {
            reason,
            op: None,
            node: None,
        }
    }

    #[must_use]
    pub const fn on(reason: Reason, node: NodeId) -> Self {
        Self {
            reason,
            op: None,
            node: Some(node),
        }
    }

    #[must_use]
    pub const fn at(mut self, op: usize) -> Self {
        if self.op.is_none() {
            self.op = Some(op);
        }
        self
    }
}

impl fmt::Display for Reject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.reason)?;
        if let Some(node) = self.node {
            write!(f, " at node {}", node.0)?;
        }
        if let Some(op) = self.op {
            write!(f, " (op {op})")?;
        }
        Ok(())
    }
}

impl std::error::Error for Reject {}
