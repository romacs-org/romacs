//! The host store: leases, collection, budget, reset, cross-scene references.

use std::sync::Arc;
use std::time::Duration;

use kurbo::{Point, Size};
use romacs_scene::diff::{Diff, Op, Reason, Rev};
use romacs_scene::fixtures::{self, FixedMeasure, FixtureResources, linear};
use romacs_scene::host::{Host, PlacementId};
use romacs_scene::motion::{Animate, Repeat};
use romacs_scene::node::{
    Cells, Channel, Dim, ImageNode, ImageSource, Kind, NodeId, NodeSpec, RectNode, SceneId,
    SceneRef, ShaderNode,
};
use romacs_scene::prop::{Prop, Value};

const MB: u64 = 1 << 20;
const P: PlacementId = PlacementId(1);

fn insert(node: NodeSpec) -> Diff {
    Diff {
        base: Rev(0),
        ops: vec![Op::Insert {
            parent: NodeId::ROOT,
            index: 0,
            node,
        }],
    }
}

fn rect() -> Diff {
    insert(NodeSpec::new(1, Kind::Rect(RectNode::default())))
}

fn image_node(id: u32, scene: SceneId, generation: u32) -> NodeSpec {
    let source = ImageSource::Scene(SceneRef {
        scene,
        generation,
        node: NodeId::ROOT,
    });
    NodeSpec::new(
        id,
        Kind::Image(ImageNode {
            source,
            fit: Default::default(),
        }),
    )
}

fn image_of(scene: SceneId, generation: u32) -> Diff {
    insert(image_node(1, scene, generation))
}

fn generation(host: &Host, id: SceneId) -> u32 {
    host.scene(id).unwrap().generation
}

#[test]
fn destroy_frees_now_without_leases_and_late_with_one() {
    let mut host = Host::new(256 * MB);
    host.create(SceneId(1), &rect()).unwrap();
    assert_eq!(host.destroy(SceneId(1)), vec![SceneId(1)]);
    assert_eq!(host.pool().used(), 0);

    host.create(SceneId(2), &rect()).unwrap();
    host.place(SceneId(2), P, Size::new(100.0, 100.0), 1.0)
        .unwrap();
    host.present(SceneId(2), P, 7, &mut FixedMeasure, &mut FixtureResources)
        .unwrap();
    assert_eq!(
        host.destroy(SceneId(2)),
        vec![],
        "frame 7 can still draw it"
    );
    assert!(host.scene(SceneId(2)).is_some() && !host.is_live(SceneId(2)));
    let update = Diff {
        base: Rev(1),
        ops: vec![],
    };
    assert_eq!(
        host.update(SceneId(2), &update).unwrap_err().reason,
        Reason::SceneNotLive(SceneId(2))
    );
    assert_eq!(host.retire(SceneId(2), P, 7), vec![SceneId(2)]);
    assert_eq!(host.pool().used(), 0);
}

#[test]
fn a_sampled_scene_survives_until_its_consumer_goes() {
    let mut host = Host::new(256 * MB);
    host.create(SceneId(1), &rect()).unwrap();
    host.create(
        SceneId(2),
        &image_of(SceneId(1), generation(&host, SceneId(1))),
    )
    .unwrap();
    assert_eq!(host.destroy(SceneId(1)), vec![], "scene 2 samples it");
    // The existing reference stays valid, but a node newly naming a dead
    // scene is refused.
    host.create(
        SceneId(3),
        &image_of(SceneId(1), generation(&host, SceneId(1))),
    )
    .unwrap_err();
    assert_eq!(
        host.destroy(SceneId(2)),
        vec![SceneId(1), SceneId(2)],
        "freed together"
    );
    assert_eq!(host.retained().count(), 0);
}

#[test]
fn a_reference_to_another_incarnation_is_refused() {
    let mut host = Host::new(256 * MB);
    host.create(SceneId(1), &rect()).unwrap();
    let old = generation(&host, SceneId(1));
    host.destroy(SceneId(1));
    host.create(SceneId(1), &rect()).unwrap();
    let r = host
        .create(SceneId(2), &image_of(SceneId(1), old))
        .unwrap_err();
    assert_eq!((r.reason, r.node), (Reason::BadReference, Some(NodeId(1))));
    let fresh = generation(&host, SceneId(1));
    host.create(SceneId(2), &image_of(SceneId(1), fresh))
        .unwrap();
}

#[test]
fn cross_scene_cycles_are_refused_with_nothing_changed() {
    let mut host = Host::new(256 * MB);
    host.create(SceneId(1), &rect()).unwrap();
    host.create(
        SceneId(2),
        &image_of(SceneId(1), generation(&host, SceneId(1))),
    )
    .unwrap();
    let before = host.clone();
    let back = Diff {
        base: Rev(1),
        ops: vec![Op::Insert {
            parent: NodeId::ROOT,
            index: 0,
            node: image_node(9, SceneId(2), generation(&host, SceneId(2))),
        }],
    };
    let r = host.update(SceneId(1), &back).unwrap_err();
    assert!(
        matches!(r.reason, Reason::Cycle(ref c) if c.len() == 2),
        "{r:?}"
    );
    assert_eq!(host, before);
    assert_eq!(
        host.render_order(),
        vec![(SceneId(1), NodeId::ROOT), (SceneId(2), NodeId::ROOT)]
    );
}

#[test]
fn over_budget_create_and_place_signal_the_shortfall() {
    let mut host = Host::new(MB);
    let big = Cells {
        cols: 512,
        rows: 512,
        glyphs: vec!['x'; 512 * 512].into(),
        shades: Arc::from([]),
        palette: Arc::from([fixtures::ACCENT]),
        font: romacs_scene::node::default_font(),
    };
    let r = host
        .create(SceneId(1), &insert(NodeSpec::new(1, Kind::Cells(big))))
        .unwrap_err();
    assert!(
        matches!(r.reason, Reason::OverBudget { needed, available } if needed > MB && available == MB)
    );
    assert_eq!(host.retained().count(), 0);

    host.create(SceneId(2), &rect()).unwrap();
    // 1000×1000 at RGBA8 plus 4× MSAA is 20 MB.
    let r = host
        .place(SceneId(2), P, Size::new(1000.0, 1000.0), 1.0)
        .unwrap_err();
    assert!(matches!(r.reason, Reason::OverBudget { .. }));
    assert!(
        host.placements(SceneId(2)).is_empty(),
        "a refused placement is not shown"
    );
    host.place(SceneId(2), P, Size::new(100.0, 100.0), 1.0)
        .unwrap();
    assert!(host.pool().used() >= 200_000);
}

#[test]
fn reset_keeps_every_scene_and_drops_frames() {
    let mut host = Host::new(256 * MB);
    host.create(SceneId(1), &rect()).unwrap();
    host.create(SceneId(2), &rect()).unwrap();
    host.place(SceneId(2), P, Size::new(50.0, 50.0), 1.0)
        .unwrap();
    host.present(SceneId(2), P, 1, &mut FixedMeasure, &mut FixtureResources)
        .unwrap();
    host.destroy(SceneId(2));
    assert_eq!(
        host.reset(),
        vec![SceneId(1)],
        "the leased-only scene goes with its frames"
    );
    assert_eq!(host.epoch(), 1);
    assert!(host.is_live(SceneId(1)));
}

#[test]
fn hit_testing_answers_from_the_presented_frame() {
    let mut host = Host::new(256 * MB);
    let card = NodeSpec::new(1, Kind::Rect(RectNode::default())).with(|c| {
        c.width = Dim::Px(50.0);
        c.height = Dim::Px(50.0);
    });
    host.create(SceneId(1), &insert(card)).unwrap();
    host.place(SceneId(1), P, Size::new(300.0, 100.0), 1.0)
        .unwrap();
    host.tick(Duration::ZERO, &|_| true);
    host.present(SceneId(1), P, 1, &mut FixedMeasure, &mut FixtureResources)
        .unwrap();
    let glide = Diff {
        base: Rev(1),
        ops: vec![Op::Set {
            id: NodeId(1),
            prop: Prop::X,
            value: Some(Value::Num(200.0)),
            animate: Some(Animate::Motion {
                spec: linear(1000),
                repeat: Repeat::Once,
            }),
        }],
    };
    host.update(SceneId(1), &glide).unwrap();
    // Not presented yet: the click lands where frame 1 drew the card.
    assert_eq!(
        host.hit(SceneId(1), P, Point::new(10.0, 10.0)).map(|h| h.0),
        Some(NodeId(1))
    );
    host.tick(Duration::from_millis(500), &|_| true);
    host.present(SceneId(1), P, 2, &mut FixedMeasure, &mut FixtureResources)
        .unwrap();
    assert_eq!(host.hit(SceneId(1), P, Point::new(10.0, 10.0)), None);
    assert_eq!(
        host.hit(SceneId(1), P, Point::new(110.0, 10.0))
            .map(|h| h.0),
        Some(NodeId(1))
    );
    assert_eq!(host.animating(), vec![SceneId(1)]);
}

#[test]
fn two_placements_report_two_boxes() {
    let mut host = Host::new(256 * MB);
    host.create(SceneId(1), &fixtures::dashboard(2, 2)).unwrap();
    host.place(SceneId(1), PlacementId(1), Size::new(300.0, 80.0), 1.0)
        .unwrap();
    host.place(SceneId(1), PlacementId(2), Size::new(600.0, 160.0), 2.0)
        .unwrap();
    assert_eq!(
        host.placements(SceneId(1)),
        vec![PlacementId(1), PlacementId(2)]
    );
    let a = host
        .layout(SceneId(1), PlacementId(1), &mut FixedMeasure)
        .unwrap();
    let b = host
        .layout(SceneId(1), PlacementId(2), &mut FixedMeasure)
        .unwrap();
    assert_ne!(a.boxes[&NodeId(1100)], b.boxes[&NodeId(1100)]);
}

#[test]
fn channels_to_another_scene_are_dependencies() {
    let mut host = Host::new(256 * MB);
    host.create(SceneId(1), &rect()).unwrap();
    let g = generation(&host, SceneId(1));
    let shader = NodeSpec::new(
        1,
        Kind::Shader(ShaderNode {
            channels: vec![Channel::Scene(SceneRef {
                scene: SceneId(1),
                generation: g,
                node: NodeId::ROOT,
            })],
            ..ShaderNode::default()
        }),
    );
    host.create(SceneId(2), &insert(shader)).unwrap();
    assert_eq!(host.destroy(SceneId(1)), vec![]);
    assert_eq!(host.destroy(SceneId(2)), vec![SceneId(1), SceneId(2)]);
}

fn target_group(id: u32, w: Dim, h: Dim) -> NodeSpec {
    NodeSpec::new(
        id,
        Kind::Group {
            flex: None,
            target: true,
        },
    )
    .with(|c| {
        c.width = w;
        c.height = h;
    })
}

fn image_of_node(id: u32, scene: SceneId, generation: u32, node: u32) -> NodeSpec {
    let source = ImageSource::Scene(SceneRef {
        scene,
        generation,
        node: NodeId(node),
    });
    NodeSpec::new(
        id,
        Kind::Image(ImageNode {
            source,
            fit: Default::default(),
        }),
    )
}

#[test]
fn a_leased_frame_keeps_what_it_sampled_after_the_reference_goes() {
    let mut host = Host::new(256 * MB);
    host.create(SceneId(1), &rect()).unwrap();
    host.create(
        SceneId(2),
        &image_of(SceneId(1), generation(&host, SceneId(1))),
    )
    .unwrap();
    host.place(SceneId(2), P, Size::new(64.0, 64.0), 1.0)
        .unwrap();
    host.present(SceneId(2), P, 5, &mut FixedMeasure, &mut FixtureResources)
        .unwrap();
    assert_eq!(host.destroy(SceneId(1)), vec![]);
    let drop_ref = Diff {
        base: Rev(1),
        ops: vec![Op::Remove { id: NodeId(1) }],
    };
    host.update(SceneId(2), &drop_ref).unwrap();
    assert_eq!(host.collect(), vec![], "frame 5 still samples scene 1");
    assert_eq!(host.retire(SceneId(2), P, 5), vec![SceneId(1)]);
}

#[test]
fn an_unplaced_frame_keeps_its_bytes_until_retired() {
    let mut host = Host::new(256 * MB);
    host.create(SceneId(1), &rect()).unwrap();
    host.place(SceneId(1), P, Size::new(100.0, 100.0), 1.0)
        .unwrap();
    host.present(SceneId(1), P, 1, &mut FixedMeasure, &mut FixtureResources)
        .unwrap();
    let shown = host.pool().used();
    host.unplace(SceneId(1), P);
    assert_eq!(host.pool().used(), shown);
    host.retire(SceneId(1), P, 1);
    assert!(host.pool().used() < shown);
}

#[test]
fn a_fractional_target_reserves_against_its_parents_size() {
    let mut host = Host::new(MB);
    let parent = NodeSpec::new(1, Kind::group()).with(|c| {
        c.width = Dim::Px(1000.0);
        c.height = Dim::Px(1000.0);
    });
    let full = parent.child(target_group(2, Dim::Fraction(1.0), Dim::Fraction(1.0)));
    host.create(SceneId(1), &insert(full)).unwrap();
    let r = host
        .place(SceneId(1), P, Size::new(100.0, 100.0), 1.0)
        .unwrap_err();
    assert!(
        matches!(r.reason, Reason::OverBudget { needed, .. } if needed > 20_000_000),
        "{r:?}"
    );
}

#[test]
fn a_sampled_target_cannot_be_removed_from_under_its_consumer() {
    let mut host = Host::new(256 * MB);
    host.create(
        SceneId(1),
        &insert(target_group(3, Dim::Px(10.0), Dim::Px(10.0))),
    )
    .unwrap();
    let g = generation(&host, SceneId(1));
    host.create(SceneId(2), &insert(image_of_node(1, SceneId(1), g, 3)))
        .unwrap();
    let remove = Diff {
        base: Rev(1),
        ops: vec![Op::Remove { id: NodeId(3) }],
    };
    let r = host.update(SceneId(1), &remove).unwrap_err();
    assert_eq!((r.reason, r.node), (Reason::BadReference, Some(NodeId(3))));
    let untarget = Diff {
        base: Rev(1),
        ops: vec![Op::Set {
            id: NodeId(3),
            prop: Prop::Target,
            value: Some(Value::Bool(false)),
            animate: None,
        }],
    };
    assert_eq!(
        host.update(SceneId(1), &untarget).unwrap_err().reason,
        Reason::BadReference
    );
}

#[test]
fn scenes_sampling_each_other_are_freed_together() {
    let mut host = Host::new(256 * MB);
    let (a, b) = (SceneId(1), SceneId(2));
    host.create(a, &insert(target_group(3, Dim::Px(10.0), Dim::Px(10.0))))
        .unwrap();
    host.create(b, &insert(target_group(3, Dim::Px(10.0), Dim::Px(10.0))))
        .unwrap();
    let (ga, gb) = (generation(&host, a), generation(&host, b));
    // Each root samples the other's independent target: no cycle of targets.
    let sample = |scene, generation| Diff {
        base: Rev(1),
        ops: vec![Op::Insert {
            parent: NodeId::ROOT,
            index: 1,
            node: image_of_node(9, scene, generation, 3),
        }],
    };
    host.update(a, &sample(b, gb)).unwrap();
    host.update(b, &sample(a, ga)).unwrap();
    assert_eq!(host.destroy(a), vec![]);
    assert_eq!(host.destroy(b), vec![a, b]);
}

#[test]
fn a_leased_frame_stays_charged_at_what_it_drew() {
    let mut host = Host::new(256 * MB);
    let group = target_group(3, Dim::Px(100.0), Dim::Px(100.0));
    host.create(SceneId(1), &insert(group)).unwrap();
    host.place(SceneId(1), P, Size::new(100.0, 100.0), 1.0)
        .unwrap();
    host.present(SceneId(1), P, 1, &mut FixedMeasure, &mut FixtureResources)
        .unwrap();
    // Two 100×100 targets at RGBA8 plus 4× MSAA.
    let drawn = host.pool().used();
    assert_eq!(drawn, 400_000);
    // Shrinking the showing or removing the target leaves frame 1's bytes.
    host.place(SceneId(1), P, Size::new(1.0, 1.0), 1.0).unwrap();
    let remove = Diff {
        base: Rev(1),
        ops: vec![Op::Remove { id: NodeId(3) }],
    };
    host.update(SceneId(1), &remove).unwrap();
    assert_eq!(host.pool().used(), drawn);
    host.retire(SceneId(1), P, 1);
    assert_eq!(host.pool().used(), 20);
}

#[test]
fn a_frame_reserves_its_motion_sampled_target_sizes() {
    let mut host = Host::new(256 * MB);
    let group = target_group(3, Dim::Px(100.0), Dim::Px(100.0));
    host.create(SceneId(1), &insert(group)).unwrap();
    host.place(SceneId(1), P, Size::new(10.0, 10.0), 1.0)
        .unwrap();
    host.tick(Duration::ZERO, &|_| true);
    let shrink = |prop| Op::Set {
        id: NodeId(3),
        prop,
        value: Some(Value::Dim(Dim::Px(1.0))),
        animate: Some(Animate::Motion {
            spec: linear(1000),
            repeat: Repeat::Once,
        }),
    };
    let diff = Diff {
        base: Rev(1),
        ops: vec![shrink(Prop::Width), shrink(Prop::Height)],
    };
    host.update(SceneId(1), &diff).unwrap();
    let list = host
        .present(SceneId(1), P, 1, &mut FixedMeasure, &mut FixtureResources)
        .unwrap();
    assert_eq!(
        list.targets[0].size,
        Size::new(100.0, 100.0),
        "drawn at the start of the glide"
    );
    assert!(host.pool().used() >= 200_000, "{}", host.pool().used());
}

#[test]
fn distinct_targets_of_outstanding_frames_are_charged_separately() {
    let mut host = Host::new(250_000);
    let a = target_group(3, Dim::Px(100.0), Dim::Px(100.0));
    host.create(SceneId(1), &insert(a)).unwrap();
    host.place(SceneId(1), P, Size::new(1.0, 1.0), 1.0).unwrap();
    host.present(SceneId(1), P, 1, &mut FixedMeasure, &mut FixtureResources)
        .unwrap();
    assert_eq!(host.pool().used(), 200_020);
    // Replace target A with an equal target B while frame 1 still draws A.
    let swap = Diff {
        base: Rev(1),
        ops: vec![
            Op::Remove { id: NodeId(3) },
            Op::Insert {
                parent: NodeId::ROOT,
                index: 0,
                node: target_group(4, Dim::Px(100.0), Dim::Px(100.0)),
            },
        ],
    };
    // The next frame alone needs B as well as frame 1's A: over the pool.
    let r = host.update(SceneId(1), &swap).unwrap_err();
    assert!(
        matches!(
            r.reason,
            Reason::OverBudget {
                needed: 400_020,
                ..
            }
        ),
        "{r:?}"
    );
    host.retire(SceneId(1), P, 1);
    host.update(SceneId(1), &swap).unwrap();
    host.present(SceneId(1), P, 2, &mut FixedMeasure, &mut FixtureResources)
        .unwrap();
    assert_eq!(host.pool().used(), 200_020);
}
