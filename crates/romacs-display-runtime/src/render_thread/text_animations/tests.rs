use super::*;
use romacs_display_protocol::text_animation::TextAnimationKind;

fn logo(moving: bool, epoch: u64) -> TextAnimation {
    TextAnimation { window: 1, top: 0, cols: 10, rows: 4, fps: 25, moving, still: false,
                    kind: TextAnimationKind::Logo { cell_aspect: 0.5, epoch, colours: Default::default() } }
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

#[test]
fn an_animated_frame_promises_no_layout_identity() {
    let mut animations = TextAnimations::default();
    let mut frame = FrameGlyphBuffer::new();
    frame.layout_identity = 7;
    animations.paint(&mut frame);
    assert_eq!(frame.layout_identity, 7, "nothing painted over it");
    animations.replace(vec![(1, logo(true, 0))]);
    animations.paint(&mut frame);
    assert_eq!(frame.layout_identity, 0, "a retained root must not stand in for it");
}

fn thread(work: romacs_logo::working::Work, epoch: u64) -> TextAnimation {
    TextAnimation { window: 1, top: 0, cols: 12, rows: 1, fps: 30, moving: true, still: false,
                    kind: TextAnimationKind::Working { work, beats: 0, epoch, colour: [0xd4, 0x8a, 0x20] } }
}

#[test]
fn a_working_thread_moves_by_itself_and_keeps_its_motion_across_states() {
    use romacs_logo::working::Work;
    let mut animations = TextAnimations::default();
    let start = Instant::now();
    animations.replace(vec![(1, thread(Work::Running, 0))]);
    animations.tick(start);
    let first = animations.running[&1].animation.cells(0.0);
    animations.tick(start + Duration::from_millis(40));
    let moved = animations.running[&1].moved;
    assert_ne!(animations.running[&1].animation.cells(moved), first, "the render thread alone moves it");
    // Its work changes: its motion goes on; a new epoch starts it over.
    animations.replace(vec![(1, thread(Work::Waiting, 0))]);
    assert!((animations.running[&1].moved - moved).abs() < 1e-9);
    animations.replace(vec![(1, thread(Work::Waiting, 1))]);
    assert_eq!(animations.running[&1].moved, 0.0);
    // A thread is drawn as its cells, never a scene; the logo is a scene.
    assert!(!animations.running[&1].animation.kind.scened());
    assert!(logo(true, 0).kind.scened());
}
