//! Transactional diffs: whole-diff validation, stale bases, atomicity.

use std::sync::Arc;

use proptest::prelude::*;
use romacs_scene::diff::{Diff, Op, Reason, Rev};
use romacs_scene::fixtures::{self, linear};
use romacs_scene::motion::{Animate, Policy, Repeat};
use romacs_scene::node::{
    AnchorId, Channel, Dim, Flex, Kind, NodeId, NodeSpec, RectNode, SceneId, ShaderNode,
};
use romacs_scene::prop::{Prop, Value};
use romacs_scene::scene::{MAX_DEPTH, Scene};

fn empty() -> Scene {
    Scene::new(SceneId(1), 1)
}

fn rect(id: u32) -> NodeSpec {
    NodeSpec::new(id, Kind::Rect(RectNode::default()))
}

fn group(id: u32) -> NodeSpec {
    NodeSpec::new(id, Kind::group())
}

fn insert(parent: u32, index: u32, node: NodeSpec) -> Op {
    Op::Insert {
        parent: NodeId(parent),
        index,
        node,
    }
}

fn set(id: u32, prop: Prop, value: Value) -> Op {
    Op::Set {
        id: NodeId(id),
        prop,
        value: Some(value),
        animate: None,
    }
}

fn apply(scene: &Scene, ops: Vec<Op>) -> Result<Scene, romacs_scene::diff::Reject> {
    scene.apply(
        &Diff {
            base: scene.rev(),
            ops,
        },
        Policy::On,
    )
}

fn reason(scene: &Scene, ops: Vec<Op>) -> Reason {
    apply(scene, ops).expect_err("diff must be rejected").reason
}

#[test]
fn applied_diff_bumps_the_revision_once() {
    let s = apply(
        &empty(),
        vec![insert(0, 0, group(1)), insert(1, 0, rect(2))],
    )
    .unwrap();
    assert_eq!(s.rev(), Rev(1));
    assert_eq!(s.node(NodeId(1)).unwrap().children, vec![NodeId(2)]);
    assert_eq!(s.node(NodeId(2)).unwrap().parent, Some(NodeId(1)));
}

#[test]
fn stale_base_is_rejected_with_the_current_revision() {
    let s = apply(&empty(), vec![insert(0, 0, rect(1))]).unwrap();
    let stale = Diff {
        base: Rev(0),
        ops: vec![Op::Remove { id: NodeId(1) }],
    };
    let r = s.apply(&stale, Policy::On).unwrap_err();
    assert_eq!(r.reason, Reason::Stale { current: Rev(1) });
}

#[test]
fn a_late_bad_op_rejects_the_whole_diff_naming_it() {
    let s = apply(&empty(), vec![insert(0, 0, rect(1))]).unwrap();
    let before = s.clone();
    let r = apply(
        &s,
        vec![
            set(1, Prop::X, Value::Num(10.0)),
            insert(0, 1, rect(2)),
            set(9, Prop::X, Value::Num(1.0)),
        ],
    )
    .unwrap_err();
    assert_eq!(
        (r.reason, r.op, r.node),
        (Reason::UnknownNode, Some(2), Some(NodeId(9)))
    );
    assert_eq!(s, before, "a rejected diff changes nothing");
}

#[test]
fn structural_errors_are_named() {
    let s = apply(
        &empty(),
        vec![insert(0, 0, group(1)), insert(1, 0, rect(2))],
    )
    .unwrap();
    assert_eq!(reason(&s, vec![insert(0, 0, rect(2))]), Reason::DuplicateId);
    assert_eq!(
        reason(
            &s,
            vec![insert(0, 0, group(5).child(rect(6)).child(rect(6)))]
        ),
        Reason::DuplicateId
    );
    assert_eq!(reason(&s, vec![insert(2, 0, rect(3))]), Reason::NotAGroup);
    assert_eq!(
        reason(&s, vec![insert(0, 5, rect(3))]),
        Reason::IndexOutOfRange
    );
    assert_eq!(
        reason(&s, vec![Op::Remove { id: NodeId::ROOT }]),
        Reason::RootFixed
    );
    let into_self = Op::Move {
        id: NodeId(1),
        parent: NodeId(1),
        index: 0,
    };
    assert_eq!(reason(&s, vec![into_self]), Reason::MoveIntoSelf);
    assert_eq!(
        reason(&s, vec![insert(0, 0, rect(4).child(rect(5)))]),
        Reason::NotAGroup
    );
}

#[test]
fn properties_typecheck_against_the_kind() {
    let s = apply(&empty(), vec![insert(0, 0, rect(1))]).unwrap();
    assert_eq!(
        reason(&s, vec![set(1, Prop::Text, Value::Str(Arc::from("x")))]),
        Reason::PropNotOnKind(Prop::Text)
    );
    assert_eq!(
        reason(&s, vec![set(1, Prop::Radius, Value::Bool(true))]),
        Reason::WrongType(Prop::Radius)
    );
    assert_eq!(
        reason(&s, vec![set(1, Prop::Opacity, Value::Num(1.5))]),
        Reason::OutOfRange(Prop::Opacity)
    );
    assert_eq!(
        reason(&s, vec![set(1, Prop::X, Value::Num(f32::NAN))]),
        Reason::OutOfRange(Prop::X)
    );
    let bad_insert = rect(2).with(|c| c.opacity = -1.0);
    assert_eq!(
        reason(&s, vec![insert(0, 0, bad_insert)]),
        Reason::OutOfRange(Prop::Opacity)
    );
}

#[test]
fn motion_needs_interpolable_values_and_repeat_needs_a_tween() {
    let s = apply(&empty(), vec![insert(0, 0, rect(1))]).unwrap();
    let animated = |prop, value, animate| Op::Set {
        id: NodeId(1),
        prop,
        value: Some(value),
        animate: Some(animate),
    };
    let tween = Animate::Motion {
        spec: linear(100),
        repeat: Repeat::Once,
    };
    assert_eq!(
        reason(&s, vec![animated(Prop::Z, Value::Num(3.0), tween)]),
        Reason::NotAnimatable(Prop::Z)
    );
    // Auto → Px has no starting length to interpolate from.
    assert_eq!(
        reason(
            &s,
            vec![animated(Prop::Width, Value::Dim(Dim::Px(5.0)), tween)]
        ),
        Reason::NotAnimatable(Prop::Width)
    );
    let spring = romacs_display_protocol::motion_spec::MotionSpec::Spring(
        romacs_display_protocol::motion_spec::SpringSpec {
            omega: romacs_display_protocol::motion_spec::AngularFrequency::new(20.0).unwrap(),
            damping: romacs_display_protocol::motion_spec::DampingRatio::new(1.0).unwrap(),
        },
    );
    let repeated_spring = Animate::Motion {
        spec: spring,
        repeat: Repeat::Forever { alternate: true },
    };
    assert_eq!(
        reason(
            &s,
            vec![animated(Prop::X, Value::Num(3.0), repeated_spring)]
        ),
        Reason::RepeatNeedsTween(Prop::X)
    );
    let no_value = Op::Set {
        id: NodeId(1),
        prop: Prop::X,
        value: None,
        animate: Some(tween),
    };
    assert_eq!(reason(&s, vec![no_value]), Reason::MissingValue(Prop::X));
}

#[test]
fn channels_name_only_target_groups() {
    let shader = |channel| {
        NodeSpec::new(
            3,
            Kind::Shader(ShaderNode {
                channels: vec![channel],
                ..ShaderNode::default()
            }),
        )
    };
    let s = apply(
        &empty(),
        vec![insert(0, 0, rect(1)), insert(0, 1, group(2))],
    )
    .unwrap();
    assert_eq!(
        reason(&s, vec![insert(0, 0, shader(Channel::Local(NodeId(1))))]),
        Reason::BadReference
    );
    assert_eq!(
        reason(&s, vec![insert(0, 0, shader(Channel::Local(NodeId(2))))]),
        Reason::BadReference
    );
    let ok = apply(
        &s,
        vec![
            set(2, Prop::Target, Value::Bool(true)),
            insert(0, 0, shader(Channel::Local(NodeId(2)))),
        ],
    );
    assert!(ok.is_ok());
    // Removing a sampled target leaves a dangling channel: refused.
    let s = ok.unwrap();
    assert_eq!(
        reason(&s, vec![Op::Remove { id: NodeId(2) }]),
        Reason::BadReference
    );
}

#[test]
fn anchors_are_refused_inside_flex_layout() {
    let flex = NodeSpec::new(
        1,
        Kind::Group {
            flex: Some(Flex::default()),
            target: false,
        },
    );
    let anchored = rect(2).with(|c| c.anchor = Some(AnchorId(7)));
    assert_eq!(
        reason(&empty(), vec![insert(0, 0, flex.child(anchored.clone()))]),
        Reason::AnchorInFlex
    );
    assert!(apply(&empty(), vec![insert(0, 0, group(1).child(anchored))]).is_ok());
}

#[test]
fn trees_deeper_than_the_bound_are_refused() {
    let deep = (1..=MAX_DEPTH as u32 + 1)
        .rev()
        .fold(None::<NodeSpec>, |inner, id| {
            let g = group(id);
            Some(match inner {
                Some(i) => g.child(i),
                None => g,
            })
        });
    let r = apply(&empty(), vec![insert(0, 0, deep.unwrap())]).unwrap_err();
    assert_eq!(r.reason, Reason::TooDeep { limit: MAX_DEPTH });
}

#[test]
fn every_fixture_applies() {
    for (name, diff) in fixtures::per_kind() {
        assert!(empty().apply(&diff, Policy::On).is_ok(), "{name}");
    }
    assert!(
        empty()
            .apply(&fixtures::dashboard(6, 32), Policy::On)
            .is_ok()
    );
}

// ── Atomicity under random op sequences ─────────────────────────────────

fn arb_op() -> impl Strategy<Value = Op> {
    let id = 0u32..12;
    prop_oneof![
        (id.clone(), 0u32..4, id.clone(), any::<bool>()).prop_map(
            |(parent, index, new, is_group)| insert(
                parent,
                index,
                if is_group { group(new) } else { rect(new) }
            )
        ),
        id.clone().prop_map(|id| Op::Remove { id: NodeId(id) }),
        (id.clone(), id.clone(), 0u32..4).prop_map(|(id, parent, index)| Op::Move {
            id: NodeId(id),
            parent: NodeId(parent),
            index,
        }),
        (id.clone(), -50.0f32..50.0).prop_map(|(id, x)| set(id, Prop::X, Value::Num(x))),
        (id.clone(), -0.5f32..1.5).prop_map(|(id, o)| set(id, Prop::Opacity, Value::Num(o))),
        (id, any::<bool>()).prop_map(|(id, t)| set(id, Prop::Target, Value::Bool(t))),
    ]
}

/// The tree's own invariants: links agree both ways and every node reaches
/// the root.
fn well_formed(s: &Scene) -> bool {
    s.nodes().iter().all(|(id, n)| {
        let linked = n
            .children
            .iter()
            .all(|c| s.node(*c).is_some_and(|c| c.parent == Some(*id)));
        let listed = n
            .parent
            .is_none_or(|p| s.node(p).is_some_and(|p| p.children.contains(id)));
        let rooted = romacs_scene::scene::ancestors(s.nodes(), *id).last() == Some(NodeId::ROOT);
        linked && listed && rooted
    })
}

/// The tree without per-node revisions, which differ between one diff and
/// the same ops as several.
type Shape = (NodeId, Kind, Option<NodeId>, Vec<NodeId>, f32, f32);

fn shape(s: &Scene) -> Vec<Shape> {
    s.nodes()
        .iter()
        .map(|(id, n)| {
            (
                *id,
                n.kind.clone(),
                n.parent,
                n.children.clone(),
                n.common.transform.x,
                n.common.opacity,
            )
        })
        .collect()
}

proptest! {
    #[test]
    fn a_diff_applies_entirely_or_not_at_all(seed in prop::collection::vec(arb_op(), 0..8),
                                             diff in prop::collection::vec(arb_op(), 1..8)) {
        // Build a starting scene from whichever seed ops apply one by one.
        let start = seed.into_iter().fold(empty(), |s, op| apply(&s, vec![op]).unwrap_or(s));
        let before = start.clone();
        match apply(&start, diff.clone()) {
            Ok(next) => {
                prop_assert_eq!(next.rev(), Rev(start.rev().0 + 1));
                prop_assert!(well_formed(&next));
                // Applying op by op reaches the same tree.
                let stepwise = diff.iter().try_fold(start.clone(), |s, op| apply(&s, vec![op.clone()]));
                prop_assert_eq!(stepwise.map(|s| shape(&s)).ok(), Some(shape(&next)));
            }
            Err(_) => prop_assert_eq!(&start, &before),
        }
        prop_assert!(well_formed(&start));
    }
}
