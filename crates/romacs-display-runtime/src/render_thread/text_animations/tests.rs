use super::*;
use romacs_display_protocol::text_animation::TextAnimationKind;

fn logo(moving: bool, epoch: u64) -> TextAnimation {
    TextAnimation { window: 1, top: 0, cols: 10, rows: 4, anchor: None, rise: None, fps: 25, moving, still: false,
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
    TextAnimation { window: 1, top: 0, cols: 12, rows: 1, anchor: Some(0), rise: None, fps: 30, moving: true, still: false,
                    kind: TextAnimationKind::Working { work, beats: 0, epoch, colour: [0xd4, 0x8a, 0x20], bead: [0xff; 3] } }
}

#[test]
fn a_working_thread_moves_by_itself_and_keeps_its_motion_across_states() {
    use romacs_logo::working::Work;
    let mut animations = TextAnimations::default();
    let start = Instant::now();
    animations.replace(vec![(1, thread(Work::Running, 0))]);
    animations.tick(start);
    let first = animations.running[&1].animation.cells(0.0);
    for step in 1..=10 { animations.tick(start + Duration::from_millis(40 * step)); }
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

#[test]
fn a_thread_is_placed_on_its_laid_row_and_never_over_other_text() {
    use romacs_display_protocol::face::BoxVerticalEdges;
    use romacs_display_protocol::frame_glyphs::GlyphRowRole;
    use romacs_display_protocol::text_animation::located;
    use romacs_display_protocol::types::{DisplayWindowId, FaceId};
    use romacs_logo::working::Work;
    // A blank row, a double-height row (two grid rows), a wrapped line,
    // the thread; the blank row emits no glyph.
    let rows: [(&str, u32, f32); 4] = [("a tall heading", 1, 16.0), ("a long reply line", 3, 48.0),
                                       ("that wrapped once", 4, 64.0), ("⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤⠤ running", 5, 80.0)];
    let mut frame = FrameGlyphBuffer::new();
    frame.char_width = 8.0;
    frame.set_draw_context(DisplayWindowId::new(1), GlyphRowRole::Text, None);
    for (line, grid_row, y) in rows {
        for (c, ch) in line.chars().enumerate() {
            frame.add_char_on_row(ch, (c as f32 * 8.0, 100.0 + y, grid_row), (8.0, 16.0, 12.0),
                                  FaceId::new(0), BoxVerticalEdges::Both);
        }
    }
    // The layout's rows, the text area's top at 100 px in the frame.
    let laid = [(0, 0.0, 16.0), (1, 16.0, 32.0), (16, 48.0, 16.0), (33, 64.0, 16.0), (51, 80.0, 16.0)];
    let mut animation = thread(Work::Running, 0);
    animation.anchor = Some(51);
    let placed = animation.placed(&laid, 90, 100.0).expect("on a laid row");
    assert_eq!((placed.top, placed.rise), (4, Some((180.0, 16.0))));
    // Found by its height: grid row 5 (4 from the first with glyphs),
    // past the blank and double-height rows.
    assert_eq!(located(&frame, &placed).map(|a| a.top), Some(4));
    // Scrolled past: no laid row shows it.
    animation.anchor = Some(3);
    assert_eq!(animation.placed(&[(10, 0.0, 16.0), (20, 16.0, 16.0)], 40, 100.0), None);
    // A row that is not its braille is never painted over.
    let wrong = TextAnimation { rise: Some((148.0, 16.0)), ..placed.clone() };
    assert_eq!(located(&frame, &wrong), None);
    // A taller glyph on the thread's own line moves its text down within
    // the laid row: still found.
    let tall = TextAnimation { rise: Some((176.0, 24.0)), ..placed.clone() };
    assert_eq!(located(&frame, &tall).map(|a| a.top), Some(4));
    // Motion off: the still frame shows alone.
    let still = TextAnimation { still: true, ..placed };
    assert_eq!(located(&frame, &still), None);
}
