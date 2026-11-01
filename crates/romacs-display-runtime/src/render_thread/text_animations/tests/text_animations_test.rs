use super::*;
use romacs_display_protocol::text_animation::TextAnimationKind;

fn logo(moving: bool, epoch: u64) -> TextAnimation {
    TextAnimation { window: 1, top: 0, cols: 10, rows: 4, fps: 25, moving,
                    kind: TextAnimationKind::Logo { cell_aspect: 0.5, epoch } }
}

#[test]
fn motion_is_paced_capped_and_never_caught_up() {
    let mut animations = TextAnimations::default();
    let start = Instant::now();
    animations.replace(vec![(1, logo(true, 0))]);
    assert_eq!(animations.deadline(start), Some(start));
    assert!(animations.tick(start), "the first frame is a still one");
    assert_eq!(animations.running[&1].moved, 0.0);
    assert!(!animations.tick(start + Duration::from_millis(10)), "not yet due");
    assert!(animations.tick(start + Duration::from_millis(40)));
    assert!((animations.running[&1].moved - 0.04).abs() < 1e-9);
    // A late frame advances at most 80ms.
    assert!(animations.tick(start + Duration::from_secs(5)));
    assert!((animations.running[&1].moved - 0.12).abs() < 1e-9);
    // Unfocused: no deadline; refocused: a still frame, then on.
    animations.focus(false);
    assert_eq!(animations.deadline(start + Duration::from_secs(6)), None);
    animations.focus(true);
    assert!(animations.tick(start + Duration::from_secs(60)));
    assert!((animations.running[&1].moved - 0.12).abs() < 1e-9);
}

#[test]
fn a_frame_keeps_motion_unless_it_starts_a_new_epoch_and_drops_what_it_omits() {
    let mut animations = TextAnimations::default();
    let start = Instant::now();
    animations.replace(vec![(1, logo(true, 0))]);
    animations.tick(start);
    animations.tick(start + Duration::from_millis(40));
    animations.replace(vec![(1, logo(false, 0))]);
    assert_eq!(animations.deadline(start), None, "paused");
    assert!((animations.running[&1].moved - 0.04).abs() < 1e-9);
    animations.replace(vec![(1, logo(true, 1))]);
    assert_eq!(animations.running[&1].moved, 0.0);
    animations.replace(vec![]);
    assert!(animations.is_empty());
}
