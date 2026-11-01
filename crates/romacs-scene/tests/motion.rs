//! Property motion on the scene's document clock.

use std::time::Duration;

use romacs_scene::diff::{Diff, Op};
use romacs_scene::fixtures::linear;
use romacs_scene::motion::{Animate, MediaClock, Policy, Repeat};
use romacs_scene::node::{Camera, Color, Kind, ModelNode, NodeId, NodeSpec, RectNode, SceneId};
use romacs_scene::prop::{Prop, Value};
use romacs_scene::scene::Scene;

const MS: Duration = Duration::from_millis(1);

fn scene(policy: Policy) -> Scene {
    let model = NodeSpec::new(
        2,
        Kind::Model(ModelNode {
            mesh: 1,
            camera: Camera::default(),
        }),
    );
    let ops = vec![
        Op::Insert {
            parent: NodeId::ROOT,
            index: 0,
            node: NodeSpec::new(1, Kind::Rect(RectNode::default())),
        },
        Op::Insert {
            parent: NodeId::ROOT,
            index: 1,
            node: model,
        },
    ];
    let s = Scene::new(SceneId(1), 1)
        .apply(
            &Diff {
                base: Default::default(),
                ops,
            },
            policy,
        )
        .unwrap();
    s.tick(Duration::ZERO, true)
}

fn set(
    s: &Scene,
    id: u32,
    prop: Prop,
    value: Option<Value>,
    animate: Option<Animate>,
    policy: Policy,
) -> Scene {
    let op = Op::Set {
        id: NodeId(id),
        prop,
        value,
        animate,
    };
    s.apply(
        &Diff {
            base: s.rev(),
            ops: vec![op],
        },
        policy,
    )
    .unwrap()
}

fn tween(ms: u64, repeat: Repeat) -> Option<Animate> {
    Some(Animate::Motion {
        spec: linear(ms),
        repeat,
    })
}

fn num(s: &Scene, id: u32, prop: Prop) -> f32 {
    match s.value(NodeId(id), prop) {
        Some(Value::Num(v)) => v,
        other => panic!("{other:?}"),
    }
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn clock_skips_hidden_time_and_never_catches_up() {
    let c = MediaClock::default()
        .tick(Duration::ZERO, true)
        .tick(100 * MS, true);
    assert_eq!(c.now(), 100 * MS);
    let hidden = c.tick(5000 * MS, false);
    assert_eq!(hidden.now(), 100 * MS);
    // The first visible frame after the gap starts counting again from there.
    let back = hidden.tick(9000 * MS, true).tick(9016 * MS, true);
    assert_eq!(back.now(), 116 * MS);
}

#[test]
fn the_target_is_visible_at_once_and_the_drawn_value_glides() {
    let s = set(
        &scene(Policy::On),
        1,
        Prop::X,
        Some(Value::Num(100.0)),
        tween(400, Repeat::Once),
        Policy::On,
    );
    assert_eq!(
        romacs_scene::prop::get(s.node(NodeId(1)).unwrap(), Prop::X),
        Some(Value::Num(100.0))
    );
    assert!(close(num(&s, 1, Prop::X), 0.0));
    let s = s.tick(100 * MS, true);
    assert!(close(num(&s, 1, Prop::X), 25.0));
    assert!(s.animating());
    let s = s.tick(400 * MS, true);
    assert!(close(num(&s, 1, Prop::X), 100.0));
    assert!(!s.animating(), "a settled motion costs nothing");
}

#[test]
fn colours_interpolate_without_quantising() {
    let red = Color::rgba(1.0, 0.0, 0.0, 1.0);
    let s = set(
        &scene(Policy::On),
        1,
        Prop::Fill,
        Some(Value::Color(red)),
        tween(100, Repeat::Once),
        Policy::On,
    );
    let s = s.tick(50 * MS, true);
    let Some(Value::Color(c)) = s.value(NodeId(1), Prop::Fill) else {
        panic!()
    };
    assert!(close(c.r, 0.5) && close(c.a, 0.5), "{c:?}");
}

#[test]
fn a_set_takes_over_from_the_presented_value() {
    let s = set(
        &scene(Policy::On),
        1,
        Prop::X,
        Some(Value::Num(100.0)),
        tween(400, Repeat::Once),
        Policy::On,
    );
    let s = s.tick(200 * MS, true);
    let s = set(
        &s,
        1,
        Prop::X,
        Some(Value::Num(0.0)),
        tween(100, Repeat::Once),
        Policy::On,
    );
    assert!(
        close(num(&s, 1, Prop::X), 50.0),
        "starts where it was drawn"
    );
    let s = s.tick(250 * MS, true);
    assert!(close(num(&s, 1, Prop::X), 25.0));
    // A plain set snaps and cancels the motion.
    let s = set(&s, 1, Prop::X, Some(Value::Num(7.0)), None, Policy::On);
    assert!(close(num(&s, 1, Prop::X), 7.0) && !s.animating());
}

#[test]
fn an_alternating_repeat_goes_back_between_legs_and_ends_on_the_target() {
    let s = set(
        &scene(Policy::On),
        1,
        Prop::X,
        Some(Value::Num(10.0)),
        tween(
            100,
            Repeat::Times {
                count: 2,
                alternate: true,
            },
        ),
        Policy::On,
    );
    let at = |ms: u32| num(&s.tick(ms * MS, true), 1, Prop::X);
    assert!(close(at(50), 5.0));
    assert!(close(at(175), 2.5), "the second leg runs back");
    assert!(
        close(at(250), 5.0),
        "the third leg runs to the target again"
    );
    assert!(s.tick(299 * MS, true).animating());
    let done = s.tick(300 * MS, true);
    assert!(!done.animating());
    assert!(
        close(num(&done, 1, Prop::X), 10.0),
        "it ends on the value Lisp set"
    );
}

#[test]
fn repeat_forever_never_settles() {
    let s = set(
        &scene(Policy::On),
        1,
        Prop::Opacity,
        Some(Value::Num(0.0)),
        tween(100, Repeat::Forever { alternate: false }),
        Policy::On,
    );
    let s = s.tick(1025 * MS, true);
    assert!(s.animating());
    assert!(close(num(&s, 1, Prop::Opacity), 0.75));
}

#[test]
fn a_drive_turns_wraps_pauses_and_resumes_without_catching_up() {
    let drive = Some(Animate::Drive { rate: 7.2 });
    let s = set(
        &scene(Policy::On),
        2,
        Prop::CameraYaw,
        Some(Value::Num(350.0)),
        drive,
        Policy::On,
    );
    let s = s.tick(2000 * MS, true);
    assert!(close(num(&s, 2, Prop::CameraYaw), (350.0 + 14.4) % 360.0));
    // Hidden for a minute: the angle holds.
    let hidden = s.tick(62_000 * MS, false);
    assert!(close(
        num(&hidden, 2, Prop::CameraYaw),
        num(&s, 2, Prop::CameraYaw)
    ));
    // Visible again: it continues from its phase, one second later one second on.
    let back = hidden.tick(70_000 * MS, true).tick(71_000 * MS, true);
    assert!(close(
        num(&back, 2, Prop::CameraYaw),
        (350.0 + 21.6) % 360.0
    ));
    assert!(back.animating(), "a drive never settles");
}

#[test]
fn a_drive_without_a_value_starts_from_the_presented_one() {
    let s = set(
        &scene(Policy::On),
        2,
        Prop::CameraYaw,
        Some(Value::Num(90.0)),
        None,
        Policy::On,
    );
    let s = set(
        &s,
        2,
        Prop::CameraYaw,
        None,
        Some(Animate::Drive { rate: 10.0 }),
        Policy::On,
    );
    let s = s.tick(1000 * MS, true);
    assert!(close(num(&s, 2, Prop::CameraYaw), 100.0));
}

#[test]
fn reduced_fades_opacity_and_snaps_geometry_and_drives() {
    let s = set(
        &scene(Policy::Reduced),
        1,
        Prop::X,
        Some(Value::Num(100.0)),
        tween(400, Repeat::Once),
        Policy::Reduced,
    );
    assert!(close(num(&s, 1, Prop::X), 100.0) && !s.animating());
    let s = set(
        &s,
        1,
        Prop::Opacity,
        Some(Value::Num(0.0)),
        tween(400, Repeat::Once),
        Policy::Reduced,
    );
    let mid = s.tick(45 * MS, true);
    assert!(close(num(&mid, 1, Prop::Opacity), 0.5), "a 90 ms fade");
    assert!(!s.tick(90 * MS, true).animating());
    let s = set(
        &s,
        2,
        Prop::CameraYaw,
        Some(Value::Num(5.0)),
        Some(Animate::Drive { rate: 7.2 }),
        Policy::Reduced,
    );
    assert!(close(
        num(&s.tick(5000 * MS, true), 2, Prop::CameraYaw),
        5.0
    ));
}

#[test]
fn off_snaps_everything_and_a_policy_change_settles_running_motion() {
    let s = set(
        &scene(Policy::Off),
        1,
        Prop::Opacity,
        Some(Value::Num(0.0)),
        tween(400, Repeat::Once),
        Policy::Off,
    );
    assert!(close(num(&s, 1, Prop::Opacity), 0.0) && !s.animating());
    let s = set(
        &scene(Policy::On),
        1,
        Prop::X,
        Some(Value::Num(100.0)),
        tween(400, Repeat::Once),
        Policy::On,
    );
    let s = s.tick(100 * MS, true).retime(Policy::Off);
    assert!(!s.animating());
    assert!(
        close(num(&s, 1, Prop::X), 100.0),
        "a tween snaps to its target"
    );
    let s = set(
        &s,
        2,
        Prop::CameraYaw,
        Some(Value::Num(0.0)),
        Some(Animate::Drive { rate: 10.0 }),
        Policy::On,
    );
    let s = s.tick(1100 * MS, true).retime(Policy::Reduced);
    assert!(
        close(num(&s, 2, Prop::CameraYaw), 10.0) && !s.animating(),
        "a drive holds its angle"
    );
}
