//! `VM2GFX01`: round trips, bounds, totality, and pinned fixture bytes.

use std::path::PathBuf;
use std::sync::Arc;

use kurbo::BezPath;
use proptest::prelude::*;
use romacs_scene::diff::{Diff, Op, Rev};
use romacs_scene::fixtures::{self, linear};
use romacs_scene::motion::{Animate, Repeat};
use romacs_scene::node::{Color, Dim, Kind, NodeId, NodeSpec, PathNode, RectNode, SceneId};
use romacs_scene::prop::{Prop, Value};
use romacs_scene::scene::MAX_NODES;
use romacs_scene::wire::{self, MAX_MESSAGE, Message};

fn round_trip(m: &Message) -> Message {
    wire::decode(&wire::encode(m).unwrap()).unwrap()
}

#[test]
fn every_fixture_round_trips() {
    for (name, diff) in fixtures::per_kind()
        .into_iter()
        .chain([("dashboard", fixtures::dashboard(6, 32))])
    {
        let m = Message::Create {
            scene: SceneId(3),
            diff,
        };
        assert_eq!(round_trip(&m), m, "{name}");
    }
}

#[test]
fn updates_with_motion_and_destroy_round_trip() {
    let ops = vec![
        Op::Set {
            id: NodeId(2),
            prop: Prop::Uniform(3),
            value: Some(Value::Num(0.5)),
            animate: Some(Animate::Motion {
                spec: linear(250),
                repeat: Repeat::Times {
                    count: 4,
                    alternate: true,
                },
            }),
        },
        Op::Set {
            id: NodeId(9),
            prop: Prop::CameraYaw,
            value: None,
            animate: Some(Animate::Drive { rate: 7.2 }),
        },
        Op::Set {
            id: NodeId(2),
            prop: Prop::Width,
            value: Some(Value::Dim(Dim::Fraction(0.5))),
            animate: None,
        },
        Op::Move {
            id: NodeId(4),
            parent: NodeId(1),
            index: 2,
        },
        Op::Remove { id: NodeId(5) },
    ];
    let m = Message::Update {
        scene: SceneId(1),
        diff: Diff { base: Rev(41), ops },
    };
    assert_eq!(round_trip(&m), m);
    let d = Message::Destroy { scene: SceneId(8) };
    assert_eq!(round_trip(&d), d);
}

#[test]
fn bounds_are_refused_naming_the_node() {
    let mut too_big = BezPath::new();
    too_big.move_to((0.0, 0.0));
    (0..MAX_MESSAGE / 8).for_each(|i| too_big.line_to((i as f64, 1.0)));
    let path = NodeSpec::new(
        77,
        Kind::Path(PathNode {
            data: Arc::new(too_big),
            ..PathNode::default()
        }),
    );
    let m = Message::Create {
        scene: SceneId(1),
        diff: Diff {
            base: Rev(0),
            ops: vec![Op::Insert {
                parent: NodeId::ROOT,
                index: 0,
                node: path,
            }],
        },
    };
    let e = wire::encode(&m).unwrap_err();
    assert_eq!((e.what, e.node), ("message over 4 MiB", Some(NodeId(77))));

    let wide = (1..=MAX_NODES as u32 + 1).fold(NodeSpec::new(1_000_000, Kind::group()), |g, i| {
        g.child(NodeSpec::new(i, Kind::group()))
    });
    let m = Message::Create {
        scene: SceneId(1),
        diff: Diff {
            base: Rev(0),
            ops: vec![Op::Insert {
                parent: NodeId::ROOT,
                index: 0,
                node: wide,
            }],
        },
    };
    let bytes = wire::encode(&m).unwrap();
    let e = wire::decode(&bytes).unwrap_err();
    assert_eq!(e.what, "more than 65536 nodes");
    assert_eq!(e.node, Some(NodeId(MAX_NODES as u32)));
}

#[test]
fn malformed_input_is_refused_not_trusted() {
    assert_eq!(
        wire::decode(b"VM2GFX02\x03\0\0\0\0").unwrap_err().what,
        "not VM2GFX01"
    );
    assert_eq!(
        wire::decode(b"VM2GFX01\x03\0\0").unwrap_err().what,
        "truncated"
    );
    assert_eq!(
        wire::decode(b"VM2GFX01\x03\0\0\0\0\0").unwrap_err().what,
        "trailing bytes"
    );
    // A count claiming more items than bytes remain is refused before allocating.
    let mut huge = b"VM2GFX01\x02\x01\0\0\0".to_vec();
    huge.extend_from_slice(&0u64.to_le_bytes());
    huge.extend_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        wire::decode(&huge).unwrap_err().what,
        "count exceeds message"
    );
}

#[test]
fn a_tween_too_long_for_a_duration_is_refused_not_a_panic() {
    let mut bytes = wire::encode(&Message::Update {
        scene: SceneId(1),
        diff: Diff {
            base: Rev(0),
            ops: vec![Op::Set {
                id: NodeId(1),
                prop: Prop::X,
                value: None,
                animate: Some(Animate::Motion {
                    spec: linear(100),
                    repeat: Repeat::Once,
                }),
            }],
        },
    })
    .unwrap();
    let secs = 0.1f32.to_le_bytes();
    let at = bytes.windows(4).position(|w| w == secs).unwrap();
    bytes[at..at + 4].copy_from_slice(&f32::MAX.to_le_bytes());
    assert_eq!(
        wire::decode(&bytes).unwrap_err().what,
        "invalid motion parameter"
    );
}

#[test]
fn colours_cross_as_rgba8() {
    let fill = Color::rgba(0.2, 0.4, 0.6, 1.0);
    let node = NodeSpec::new(
        1,
        Kind::Rect(RectNode {
            fill,
            ..RectNode::default()
        }),
    );
    let m = Message::Create {
        scene: SceneId(1),
        diff: Diff {
            base: Rev(0),
            ops: vec![Op::Insert {
                parent: NodeId::ROOT,
                index: 0,
                node,
            }],
        },
    };
    let Message::Create { diff, .. } = round_trip(&m) else {
        panic!()
    };
    let Op::Insert { node, .. } = &diff.ops[0] else {
        panic!()
    };
    let Kind::Rect(r) = &node.kind else { panic!() };
    assert_eq!(
        r.fill,
        Color::rgba(51.0 / 255.0, 102.0 / 255.0, 153.0 / 255.0, 1.0)
    );
}

/// The bytes of every fixture are pinned: the codec is a protocol, so a
/// change to it must be deliberate. `ROMACS_SCENE_BLESS=1` rewrites them.
#[test]
fn fixture_bytes_are_stable() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let bless = std::env::var_os("ROMACS_SCENE_BLESS").is_some();
    for (name, diff) in fixtures::per_kind() {
        let bytes = wire::encode(&Message::Create {
            scene: SceneId(1),
            diff,
        })
        .unwrap();
        let path = dir.join(format!("{name}.vm2gfx"));
        if bless {
            std::fs::write(&path, &bytes).unwrap();
        }
        let pinned = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(bytes, pinned, "{name}");
    }
}

// ── Property tests ──────────────────────────────────────────────────────

/// `f32` values survive the wire exactly; colours survive as RGBA8.
fn arb_f32() -> impl Strategy<Value = f32> {
    (-1.0e4f32..1.0e4).prop_map(|v| (v * 8.0).round() / 8.0)
}

fn arb_color() -> impl Strategy<Value = Color> {
    any::<[u8; 4]>().prop_map(|[r, g, b, a]| {
        Color::rgba(
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
            f32::from(a) / 255.0,
        )
    })
}

fn arb_leaf() -> impl Strategy<Value = Kind> {
    prop_oneof![
        (arb_f32(), arb_color()).prop_map(|(radius, fill)| Kind::Rect(RectNode {
            radius,
            fill,
            ..RectNode::default()
        })),
        prop::collection::vec((arb_f32(), arb_f32()), 1..6).prop_map(|pts| {
            let mut p = BezPath::new();
            p.move_to((f64::from(pts[0].0), f64::from(pts[0].1)));
            pts[1..]
                .iter()
                .for_each(|(x, y)| p.line_to((f64::from(*x), f64::from(*y))));
            Kind::Path(PathNode {
                data: Arc::new(p),
                ..PathNode::default()
            })
        }),
        ".{0,12}".prop_map(|t| Kind::Text(romacs_scene::node::TextNode {
            text: Arc::from(t),
            font: romacs_scene::node::default_font(),
            color: fixtures::ACCENT,
            wrap: true,
        })),
    ]
}

fn arb_node() -> impl Strategy<Value = NodeSpec> {
    let leaf = (0u32..1000, arb_leaf(), arb_f32(), arb_f32()).prop_map(|(id, kind, x, y)| {
        NodeSpec::new(id, kind).with(|c| {
            c.transform.x = x;
            c.transform.y = y;
        })
    });
    leaf.prop_recursive(3, 24, 4, |inner| {
        (
            0u32..1000,
            prop::collection::vec(inner, 0..4),
            any::<bool>(),
        )
            .prop_map(|(id, children, target)| NodeSpec {
                children,
                ..NodeSpec::new(id, Kind::Group { flex: None, target })
            })
    })
}

proptest! {
    #[test]
    fn encode_then_decode_is_identity(node in arb_node(), base in any::<u64>(), scene in any::<u32>()) {
        let m = Message::Update {
            scene: SceneId(scene),
            diff: Diff { base: Rev(base), ops: vec![Op::Insert { parent: NodeId::ROOT, index: 0, node }] },
        };
        prop_assert_eq!(round_trip(&m), m);
    }

    #[test]
    fn decoding_any_bytes_never_panics(tail in prop::collection::vec(any::<u8>(), 0..256)) {
        let mut bytes = wire::MAGIC.to_vec();
        bytes.extend(tail);
        let _ = wire::decode(&bytes);
    }

    #[test]
    fn decoding_a_corrupted_fixture_never_panics(index in 0usize..9, at in any::<prop::sample::Index>(), byte in any::<u8>()) {
        let (_, diff) = fixtures::per_kind().swap_remove(index);
        let mut bytes = wire::encode(&Message::Create { scene: SceneId(1), diff }).unwrap();
        let i = at.index(bytes.len());
        bytes[i] = byte;
        let _ = wire::decode(&bytes);
    }
}
