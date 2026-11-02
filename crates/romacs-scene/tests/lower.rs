//! Layout, the dependency graph, lowering to the draw list, and hit testing.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use kurbo::{Point, Rect, Size};
use proptest::prelude::*;
use romacs_scene::diff::{Diff, Op};
use romacs_scene::draw::{DrawList, Paint, Source, TargetId};
use romacs_scene::fixtures::{self, FixedMeasure, FixtureResources, STEP, linear};
use romacs_scene::graph::{self, Edge};
use romacs_scene::layout::{self, Content};
use romacs_scene::lower;
use romacs_scene::motion::{Animate, Policy, Repeat};
use romacs_scene::node::{
    AnchorId, Channel, Dim, Kind, NodeId, NodeSpec, RectNode, SceneId, SceneRef, ShaderNode,
    TextNode,
};
use romacs_scene::prop::{Prop, Value};
use romacs_scene::scene::Scene;

const PLACEMENT: Size = Size::new(320.0, 200.0);

/// `draw.rs`'s contract, checked the way the renderer will refuse a list.
fn check_contract(list: &DrawList) {
    let last = list.targets.last().expect("a placement target");
    assert_eq!(last.id, TargetId(0), "the placement root is last");
    let mut seen = BTreeSet::new();
    for t in &list.targets {
        for item in &t.items {
            assert!((0.0..=1.0).contains(&item.opacity));
            if let Paint::Sample(s) = &item.paint {
                for input in std::iter::once(&Some(s.source)).chain(s.channels.iter()) {
                    if let Some(Source::Target(id)) = input {
                        assert!(seen.contains(id), "{id:?} sampled before it was rendered");
                    }
                }
            }
        }
        assert!(seen.insert(t.id), "{:?} appears twice", t.id);
    }
}

fn scene(diff: &Diff) -> Scene {
    Scene::new(SceneId(1), 1).apply(diff, Policy::On).unwrap()
}

#[test]
fn every_fixture_lowers_within_the_contract() {
    for (name, _) in fixtures::per_kind() {
        let list = fixtures::draw_list(name, PLACEMENT, 0).unwrap();
        check_contract(&list);
        let items: usize = list.targets.iter().map(|t| t.items.len()).sum();
        assert!(items > 0, "{name} draws something");
    }
}

#[test]
fn a_target_sampled_by_an_earlier_sibling_renders_first() {
    let list = fixtures::draw_list("target", PLACEMENT, 0).unwrap();
    let ids: Vec<_> = list.targets.iter().map(|t| t.id).collect();
    assert_eq!(ids, vec![TargetId(21), TargetId(0)]);
    assert_eq!(list.targets[0].size, Size::new(160.0, 200.0));
    // The word is drawn into the group's own coordinates, not the placement's.
    let word = &list.targets[0].items[0];
    assert_eq!(word.node, 22);
    assert_eq!(word.transform.translation(), kurbo::Vec2::new(30.0, 80.0));
    let composite = list.targets[1].items.iter().find(|i| i.node == 21).unwrap();
    assert!(
        matches!(&composite.paint, Paint::Sample(s) if s.source == Source::Target(TargetId(21)))
    );
    assert_eq!(
        composite.transform.translation(),
        kurbo::Vec2::new(160.0, 0.0)
    );
}

#[test]
fn the_rect_fixture_glides_at_the_step_clock() {
    let x_at = |steps| {
        let list = fixtures::draw_list("rect", PLACEMENT, steps).unwrap();
        let card = list.targets[0]
            .items
            .iter()
            .find(|i| i.node == 2)
            .unwrap()
            .clone();
        card.transform.translation().x
    };
    assert!((x_at(0) - 80.0).abs() < 1e-3);
    // 15 steps of 16.667 ms is 250 ms: half of the 500 ms glide to x = 200.
    assert!((x_at(15) - 140.0).abs() < 0.1, "{}", x_at(15));
    assert!((x_at(40) - 200.0).abs() < 1e-3);
}

#[test]
fn text_wraps_at_its_box_and_lowers_one_item_per_line() {
    let list = fixtures::draw_list("text", PLACEMENT, 0).unwrap();
    let lines: Vec<_> = list.targets[0]
        .items
        .iter()
        .filter_map(|i| match &i.paint {
            Paint::Text(t) => Some((t.text.to_string(), t.origin.y)),
            _ => None,
        })
        .collect();
    // 280 px at 9.6 px per character holds 29 characters.
    assert_eq!(
        lines,
        vec![
            ("Romacs scenes draw text".to_string(), 16.0),
            ("through the glyph atlas".to_string(), 36.0)
        ]
    );
}

#[test]
fn image_fit_contains_covers_and_fills() {
    let natural = Size::new(64.0, 48.0);
    let bounds = Size::new(200.0, 100.0);
    assert_eq!(
        lower::fit(romacs_scene::node::Fit::Fill, natural, bounds),
        Rect::new(0.0, 0.0, 200.0, 100.0)
    );
    let contain = lower::fit(romacs_scene::node::Fit::Contain, natural, bounds);
    assert!(
        (contain.height() - 100.0).abs() < 1e-9
            && (contain.x0 - (200.0 - 133.333_333) / 2.0).abs() < 1e-3
    );
    let cover = lower::fit(romacs_scene::node::Fit::Cover, natural, bounds);
    assert!((cover.width() - 200.0).abs() < 1e-9 && cover.y0 < 0.0);
}

#[test]
fn flex_dashboard_lays_out_columns_with_gaps() {
    let s = scene(&fixtures::dashboard(6, 32));
    let solved = layout::solve(
        &s,
        Size::new(2560.0, 1440.0),
        &BTreeMap::new(),
        &mut FixedMeasure,
    );
    let col = |c: u32| solved.boxes[&NodeId(100 + c * 1000)];
    assert!((col(0).x0 - 12.0).abs() < 1e-3);
    assert!((col(1).x0 - (col(0).x1 + 12.0)).abs() < 1e-3);
    assert!((col(5).x1 - (2560.0 - 12.0)).abs() < 1e-3, "{:?}", col(5));
    let row = |c: u32, r: u32| solved.boxes[&NodeId(101 + c * 1000 + r)];
    assert!((row(0, 1).y0 - (row(0, 0).y1 + 2.0)).abs() < 1e-3);
    assert!(matches!(solved.content.get(&NodeId(101)), Some(Content::Lines(l)) if l.len() == 1));
}

#[test]
fn one_scene_in_two_placements_has_two_layouts() {
    let s = scene(&fixtures::dashboard(2, 3));
    let small = layout::solve(
        &s,
        Size::new(300.0, 100.0),
        &BTreeMap::new(),
        &mut FixedMeasure,
    );
    let large = layout::solve(
        &s,
        Size::new(900.0, 300.0),
        &BTreeMap::new(),
        &mut FixedMeasure,
    );
    assert_ne!(small.boxes[&NodeId(1100)], large.boxes[&NodeId(1100)]);
    assert_eq!(small.boxes[&NodeId(1)].size(), Size::new(300.0, 100.0));
    assert_eq!(large.boxes[&NodeId(1)].size(), Size::new(900.0, 300.0));
}

#[test]
fn an_anchored_node_follows_its_anchor_and_hides_without_one() {
    let mark =
        NodeSpec::new(1, Kind::Rect(RectNode::default())).with(|c| c.anchor = Some(AnchorId(3)));
    let s = scene(&Diff {
        base: Default::default(),
        ops: vec![Op::Insert {
            parent: NodeId::ROOT,
            index: 0,
            node: NodeSpec::new(5, Kind::group()).child(mark),
        }],
    });
    let at = |r: Rect| BTreeMap::from([(AnchorId(3), r)]);
    let before = layout::solve(
        &s,
        PLACEMENT,
        &at(Rect::new(10.0, 20.0, 90.0, 36.0)),
        &mut FixedMeasure,
    );
    assert_eq!(
        before.in_scene(&s, NodeId(1)),
        Some(Rect::new(10.0, 20.0, 90.0, 36.0))
    );
    // The text rewrapped after a resize: the anchor moved, the mark with it.
    let after = layout::solve(
        &s,
        PLACEMENT,
        &at(Rect::new(0.0, 52.0, 120.0, 72.0)),
        &mut FixedMeasure,
    );
    assert_eq!(
        after.in_scene(&s, NodeId(1)),
        Some(Rect::new(0.0, 52.0, 120.0, 72.0))
    );
    let gone = layout::solve(&s, PLACEMENT, &BTreeMap::new(), &mut FixedMeasure);
    assert_eq!(gone.boxes.get(&NodeId(1)), None);
}

#[test]
fn a_click_mid_glide_hits_the_drawn_position() {
    let s = fixtures::scene_at(&fixtures::per_kind()[0].1, Some(fixtures::glide), 15).unwrap();
    let solved = layout::solve(&s, PLACEMENT, &BTreeMap::new(), &mut FixedMeasure);
    // Drawn at x = 140 (target 200); the card is 160 wide.
    let (node, local) = romacs_scene::hit::hit(&s, &solved, Point::new(145.0, 60.0)).unwrap();
    assert_eq!(node, NodeId(2));
    assert!((local.x - 5.0).abs() < 0.1, "{local:?}");
    // Its target box reaches x = 360, but it is drawn up to 300: backdrop.
    let (node, _) = romacs_scene::hit::hit(&s, &solved, Point::new(310.0, 60.0)).unwrap();
    assert_eq!(node, NodeId(1));
}

#[test]
fn hit_testing_inverts_rotation_and_respects_clips() {
    let card = NodeSpec::new(2, Kind::Rect(RectNode::default())).with(|c| {
        c.width = Dim::Px(100.0);
        c.height = Dim::Px(20.0);
        c.transform.x = 100.0;
        c.transform.y = 100.0;
        c.transform.rotate = 90.0;
    });
    let clip = NodeSpec::new(1, Kind::group()).with(|c| {
        c.width = Dim::Px(320.0);
        c.height = Dim::Px(105.0);
        c.clip = true;
    });
    let s = scene(&Diff {
        base: Default::default(),
        ops: vec![Op::Insert {
            parent: NodeId::ROOT,
            index: 0,
            node: clip.child(card),
        }],
    });
    let solved = layout::solve(&s, PLACEMENT, &BTreeMap::new(), &mut FixedMeasure);
    let hit = |x, y| romacs_scene::hit::hit(&s, &solved, Point::new(x, y)).map(|h| h.0);
    // Rotated about its centre (150, 110): now 20 wide and 100 tall.
    assert_eq!(hit(150.0, 70.0), Some(NodeId(2)));
    assert_eq!(hit(190.0, 110.0), None, "outside the rotated box");
    assert_eq!(hit(150.0, 140.0), None, "clipped by the group");
}

#[test]
fn hidden_and_transparent_nodes_neither_draw_nor_hit() {
    let s = scene(&fixtures::per_kind()[0].1);
    let hide = |prop, value| {
        s.apply(
            &Diff {
                base: s.rev(),
                ops: vec![Op::Set {
                    id: NodeId(2),
                    prop,
                    value: Some(value),
                    animate: None,
                }],
            },
            Policy::On,
        )
        .unwrap()
    };
    for hidden in [
        hide(Prop::Visible, Value::Bool(false)),
        hide(Prop::Opacity, Value::Num(0.0)),
    ] {
        let solved = layout::solve(&hidden, PLACEMENT, &BTreeMap::new(), &mut FixedMeasure);
        let list = lower::lower(&hidden, &solved, &mut FixtureResources);
        assert!(list.targets[0].items.iter().all(|i| i.node != 2));
        assert_eq!(
            romacs_scene::hit::hit(&hidden, &solved, Point::new(100.0, 70.0))
                .unwrap()
                .0,
            NodeId(1)
        );
    }
}

#[test]
fn path_revision_is_stable_and_changes_with_its_node() {
    let s = scene(&fixtures::per_kind()[1].1);
    let revision = |s: &Scene| {
        let solved = layout::solve(s, PLACEMENT, &BTreeMap::new(), &mut FixedMeasure);
        let list = lower::lower(s, &solved, &mut FixtureResources);
        list.targets[0]
            .items
            .iter()
            .find_map(|i| match &i.paint {
                Paint::Path(p) if i.node == 4 => Some(p.revision),
                _ => None,
            })
            .unwrap()
    };
    let set = |s: &Scene, prop, value| {
        s.apply(
            &Diff {
                base: s.rev(),
                ops: vec![Op::Set {
                    id: NodeId(4),
                    prop,
                    value: Some(value),
                    animate: None,
                }],
            },
            Policy::On,
        )
        .unwrap()
    };
    let r0 = revision(&s);
    assert_eq!(r0, revision(&s), "stable across frames");
    let moved = set(&s, Prop::X, Value::Num(3.0));
    assert_ne!(
        revision(&moved),
        r0,
        "any set on the node bumps its revision"
    );
    let wider = set(&s, Prop::StrokeWidth, Value::Num(4.0));
    assert_ne!(revision(&wider), r0);
}

#[test]
fn a_colour_motion_keeps_the_path_revision() {
    let s = scene(&fixtures::per_kind()[1].1).tick(Duration::ZERO, true);
    let s = s
        .apply(
            &Diff {
                base: s.rev(),
                ops: vec![Op::Set {
                    id: NodeId(4),
                    prop: Prop::Fill,
                    value: Some(Value::Color(fixtures::FOREGROUND)),
                    animate: Some(Animate::Motion {
                        spec: linear(100),
                        repeat: Repeat::Once,
                    }),
                }],
            },
            Policy::On,
        )
        .unwrap();
    let path_at = |steps| {
        let s = s.tick(STEP * steps, true);
        let solved = layout::solve(&s, PLACEMENT, &BTreeMap::new(), &mut FixedMeasure);
        let list = lower::lower(&s, &solved, &mut FixtureResources);
        list.targets[0]
            .items
            .iter()
            .find_map(|i| match &i.paint {
                Paint::Path(p) if i.node == 4 => Some(p.clone()),
                _ => None,
            })
            .unwrap()
    };
    let (early, late) = (path_at(1), path_at(4));
    assert_ne!(early.fill, late.fill, "the fill is mid-motion");
    assert_eq!(early.revision, late.revision, "colours are not tessellated");
}

#[test]
fn a_shader_binds_its_channels_in_order() {
    let (_, diff) = fixtures::per_kind()
        .into_iter()
        .find(|(n, _)| *n == "target")
        .unwrap();
    let s = scene(&diff);
    let other = SceneRef {
        scene: SceneId(9),
        generation: 1,
        node: NodeId::ROOT,
    };
    let s = s
        .apply(
            &Diff {
                base: s.rev(),
                ops: vec![Op::Set {
                    id: NodeId(20),
                    prop: Prop::Channels,
                    value: Some(Value::Channels(
                        [Channel::Scene(other), Channel::Local(NodeId(21))].into(),
                    )),
                    animate: None,
                }],
            },
            Policy::On,
        )
        .unwrap();
    let solved = layout::solve(&s, PLACEMENT, &BTreeMap::new(), &mut FixedMeasure);
    let list = lower::lower(&s, &solved, &mut FixtureResources);
    check_contract(&list);
    let shader = list.targets[1].items.iter().find(|i| i.node == 20).unwrap();
    let Paint::Sample(sample) = &shader.paint else {
        panic!("{:?}", shader.paint)
    };
    assert_eq!(sample.source, Source::Surface(1020));
    assert_eq!(
        &*sample.channels,
        &[Some(Source::Surface(2)), Some(Source::Target(TargetId(21)))],
        "another scene binds as its surface, a local group as its target"
    );
}

#[test]
fn a_200_node_dashboard_applies_and_lays_out_within_budget() {
    let diff = fixtures::dashboard(6, 32);
    let best = |f: &mut dyn FnMut()| {
        (0..20)
            .map(|_| {
                let t = Instant::now();
                f();
                t.elapsed()
            })
            .min()
            .unwrap()
    };
    let apply = best(&mut || {
        std::hint::black_box(Scene::new(SceneId(1), 1).apply(&diff, Policy::On).unwrap());
    });
    let s = Scene::new(SceneId(1), 1).apply(&diff, Policy::On).unwrap();
    let solve = best(&mut || {
        std::hint::black_box(layout::solve(
            &s,
            Size::new(2560.0, 1440.0),
            &BTreeMap::new(),
            &mut FixedMeasure,
        ));
    });
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    eprintln!(
        "200-node dashboard: apply {:.3} ms + layout {:.3} ms (best of 20)",
        ms(apply),
        ms(solve)
    );
    // §7: 0.5 ms. Wall time measures the budget only in an optimised build
    // on a quiet host, so the bound is enforced on request.
    if !cfg!(debug_assertions) && std::env::var_os("ROMACS_SCENE_TIMING").is_some() {
        assert!(
            apply + solve < Duration::from_micros(500),
            "{:?}",
            apply + solve
        );
    }
}

// ── The dependency graph ────────────────────────────────────────────────

fn target(id: u32, channels: Vec<Channel>) -> NodeSpec {
    let shader = NodeSpec::new(
        id + 100,
        Kind::Shader(ShaderNode {
            channels,
            ..ShaderNode::default()
        }),
    );
    NodeSpec::new(
        id,
        Kind::Group {
            flex: None,
            target: true,
        },
    )
    .child(shader)
}

#[test]
fn a_shader_sampling_its_own_target_is_a_cycle() {
    let diff = Diff {
        base: Default::default(),
        ops: vec![Op::Insert {
            parent: NodeId::ROOT,
            index: 0,
            node: target(1, vec![Channel::Local(NodeId(1))]),
        }],
    };
    let s = scene(&diff);
    let units = graph::units(&s);
    let cycle = graph::order(&units, &graph::edges(&s)).unwrap_err();
    assert_eq!(cycle, vec![(SceneId(1), NodeId(1))]);
}

proptest! {
    /// Any DAG over target units orders sources before consumers; adding a
    /// back edge is always reported as a cycle.
    #[test]
    fn order_is_topological(edges in prop::collection::vec((0u32..12, 0u32..12), 0..30)) {
        let unit = |n: u32| (SceneId(n % 3), NodeId(n));
        let units: BTreeSet<_> = (0..12).map(unit).collect();
        let dag: Vec<Edge> = edges
            .iter()
            .filter(|(a, b)| a > b)
            .map(|(a, b)| Edge { consumer: unit(*a), source: unit(*b) })
            .collect();
        let order = graph::order(&units, &dag).unwrap();
        let pos: BTreeMap<_, _> = order.iter().enumerate().map(|(i, u)| (*u, i)).collect();
        prop_assert_eq!(order.len(), units.len());
        for e in &dag {
            prop_assert!(pos[&e.source] < pos[&e.consumer]);
        }
        if let Some(e) = dag.first() {
            let mut cyclic = dag.clone();
            cyclic.push(Edge { consumer: e.source, source: e.consumer });
            let cycle = graph::order(&units, &cyclic).unwrap_err();
            prop_assert!(!cycle.is_empty());
            // Every step of the reported cycle is an edge.
            for (i, u) in cycle.iter().enumerate() {
                let next = cycle[(i + 1) % cycle.len()];
                prop_assert!(cyclic.iter().any(|e| e.consumer == *u && e.source == next));
            }
        }
    }
}

#[test]
fn text_nodes_measure_through_the_measure_trait() {
    let label = NodeSpec::new(
        1,
        Kind::Text(TextNode {
            text: Arc::from("abc"),
            font: romacs_scene::node::default_font(),
            color: fixtures::FOREGROUND,
            wrap: false,
        }),
    );
    let s = scene(&Diff {
        base: Default::default(),
        ops: vec![Op::Insert {
            parent: NodeId::ROOT,
            index: 0,
            node: label,
        }],
    });
    let solved = layout::solve(&s, PLACEMENT, &BTreeMap::new(), &mut FixedMeasure);
    // 3 characters at 0.6 × 14 px, one 1.25 × 14 px line.
    let b = solved.boxes[&NodeId(1)];
    assert!(
        (b.width() - 25.2).abs() < 1e-3 && (b.height() - 17.5).abs() < 1e-3,
        "{b:?}"
    );
}

#[test]
fn a_size_motion_moves_the_drawn_box_not_the_layout() {
    let s = scene(&fixtures::per_kind()[0].1).tick(Duration::ZERO, true);
    let s = s
        .apply(
            &Diff {
                base: s.rev(),
                ops: vec![Op::Set {
                    id: NodeId(2),
                    prop: Prop::Width,
                    value: Some(Value::Dim(Dim::Px(260.0))),
                    animate: Some(Animate::Motion {
                        spec: linear(100),
                        repeat: Repeat::Once,
                    }),
                }],
            },
            Policy::On,
        )
        .unwrap()
        .tick(STEP * 3, true);
    let solved = layout::solve(&s, PLACEMENT, &BTreeMap::new(), &mut FixedMeasure);
    assert_eq!(
        solved.boxes[&NodeId(2)].width(),
        260.0,
        "layout reads the target"
    );
    let list = lower::lower(&s, &solved, &mut FixtureResources);
    let card = list.targets[0].items.iter().find(|i| i.node == 2).unwrap();
    let Paint::Rect(r) = &card.paint else {
        panic!()
    };
    assert!(
        (r.rect.width() - 210.0).abs() < 0.1,
        "drawn at the sampled width: {}",
        r.rect.width()
    );
}

#[test]
fn a_root_transform_carries_the_whole_page() {
    let s = scene(&fixtures::per_kind()[0].1);
    let s = s
        .apply(
            &Diff {
                base: s.rev(),
                ops: vec![Op::Set {
                    id: NodeId::ROOT,
                    prop: Prop::X,
                    value: Some(Value::Num(30.0)),
                    animate: None,
                }],
            },
            Policy::On,
        )
        .unwrap();
    let solved = layout::solve(&s, PLACEMENT, &BTreeMap::new(), &mut FixedMeasure);
    let list = lower::lower(&s, &solved, &mut FixtureResources);
    let x = |n| {
        list.targets[0]
            .items
            .iter()
            .find(|i| i.node == n)
            .unwrap()
            .transform
            .translation()
            .x
    };
    assert_eq!((x(1), x(2)), (30.0, 110.0));
}

#[test]
fn pixels_outside_a_target_texture_are_not_hit() {
    let child = NodeSpec::new(2, Kind::Rect(RectNode::default())).with(|c| {
        c.width = Dim::Px(100.0);
        c.height = Dim::Px(100.0);
        c.transform.origin_x = 0.0;
        c.transform.origin_y = 0.0;
    });
    let group = NodeSpec::new(
        1,
        Kind::Group {
            flex: None,
            target: true,
        },
    )
    .with(|c| {
        c.width = Dim::Px(50.0);
        c.height = Dim::Px(50.0);
    })
    .child(child);
    let s = scene(&Diff {
        base: Default::default(),
        ops: vec![Op::Insert {
            parent: NodeId::ROOT,
            index: 0,
            node: group,
        }],
    });
    let solved = layout::solve(&s, PLACEMENT, &BTreeMap::new(), &mut FixedMeasure);
    let hit = |x, y| romacs_scene::hit::hit(&s, &solved, Point::new(x, y)).map(|h| h.0);
    assert_eq!(hit(25.0, 25.0), Some(NodeId(2)));
    assert_eq!(hit(75.0, 25.0), None);
}

#[test]
fn a_rotated_target_hits_only_within_its_own_texture() {
    let child = NodeSpec::new(2, Kind::Rect(RectNode::default())).with(|c| {
        c.width = Dim::Px(30.0);
        c.height = Dim::Px(10.0);
        c.transform.origin_x = 0.0;
        c.transform.origin_y = 0.0;
    });
    let group = NodeSpec::new(
        1,
        Kind::Group {
            flex: None,
            target: true,
        },
    )
    .with(|c| {
        c.width = Dim::Px(10.0);
        c.height = Dim::Px(10.0);
        c.transform.x = 100.0;
        c.transform.y = 100.0;
        c.transform.rotate = 45.0;
    })
    .child(child);
    let s = scene(&Diff {
        base: Default::default(),
        ops: vec![Op::Insert {
            parent: NodeId::ROOT,
            index: 0,
            node: group,
        }],
    });
    let solved = layout::solve(&s, PLACEMENT, &BTreeMap::new(), &mut FixedMeasure);
    // Target-local (x, 5) in placement coordinates: rotate about the centre (5, 5).
    let at = |x: f64| {
        let (sin, cos) = 45f64.to_radians().sin_cos();
        let (dx, dy) = (x - 5.0, 0.0);
        Point::new(105.0 + dx * cos - dy * sin, 105.0 + dx * sin + dy * cos)
    };
    let hit = |p| romacs_scene::hit::hit(&s, &solved, p).map(|h| h.0);
    assert_eq!(hit(at(5.0)), Some(NodeId(2)));
    assert_eq!(
        hit(at(12.0)),
        None,
        "outside the 10×10 texture, inside its bounding box"
    );
}
