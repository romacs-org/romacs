use super::*;
use romacs_display_protocol::input_progress::{InputProgress, InputStream};
use winit::keyboard::KeyCode;

const KEY: PhysicalKey = PhysicalKey::Code(KeyCode::KeyP);

#[test]
fn slow_evaluator_has_at_most_one_unread_repeat_without_repeat_debt() {
    let mut repeats = KeyRepeats::default();
    let window = WindowId::from_raw(1);
    let stream = InputStream::default();
    let first = stream.issue().unwrap();
    repeats.queued(window, KEY, Some(first.receipt()));
    for _ in 0..1000 {
        assert!(!repeats.admit(window, KEY, true));
    }
    let mut progress = InputProgress::default();
    let command = progress.begin_command();
    progress.consumed(first);
    assert!(repeats.admit(window, KEY, true));
    let next = stream.issue().unwrap();
    repeats.queued(window, KEY, Some(next.receipt()));
    assert!(!repeats.admit(window, KEY, true));
    drop(command);
    assert!(!repeats.admit(window, KEY, true));
    progress.consumed(next);
    assert!(repeats.admit(window, KEY, true));
}

#[test]
fn release_repress_and_other_physical_typing_are_lossless() {
    let mut repeats = KeyRepeats::default();
    let window = WindowId::from_raw(1);
    let old = InputStream::default().issue().unwrap();
    repeats.queued(window, KEY, Some(old.receipt()));
    assert!(repeats.admit(window, PhysicalKey::Code(KeyCode::KeyX), false));
    assert!(repeats.admit(window, KEY, false));
    repeats.release(window, KEY, std::time::Instant::now());
    assert!(!repeats.admit(window, KEY, true));
    assert!(repeats.admit(window, KEY, false));
    let new = InputStream::default().issue().unwrap();
    repeats.queued(window, KEY, Some(new.receipt()));
    drop(old); // A late old delivery cannot settle the new press.
    assert!(!repeats.admit(window, KEY, true));
    drop(new);
    assert!(repeats.admit(window, KEY, true));
}

#[test]
fn focus_loss_retires_only_its_window_and_no_orphan_repeat_is_admitted() {
    let mut repeats = KeyRepeats::default();
    let one = WindowId::from_raw(1);
    let two = WindowId::from_raw(2);
    let first = InputStream::default().issue().unwrap();
    let second = InputStream::default().issue().unwrap();
    repeats.queued(one, KEY, Some(first.receipt()));
    repeats.queued(two, KEY, Some(second.receipt()));
    repeats.retire_window(one);
    drop(first);
    assert!(!repeats.admit(one, KEY, true));
    assert!(!repeats.admit(two, KEY, true));
    drop(second);
    assert!(repeats.admit(two, KEY, true));
    assert!(repeats.admit(one, KEY, false));
}

/// The usage log's `repeat` record: a repeat answers how long after its
/// key's press it came, and the release how long the key was held, both by
/// the times the render thread saw them; a repeat of a key no press was
/// seen for, or after its release, answers nothing.
#[test]
fn holds_time_repeats_from_their_press_and_end_at_the_release() {
    use std::time::{Duration, Instant};
    let mut repeats = KeyRepeats::default();
    let window = WindowId::from_raw(1);
    let at = Instant::now();
    let ms = |n| at + Duration::from_millis(n);
    assert_eq!(repeats.pressed(window, KEY, true, at), None, "no press seen");
    assert_eq!(repeats.pressed(window, KEY, false, at), None);
    assert_eq!(repeats.pressed(window, KEY, true, ms(260)), Some(Duration::from_millis(260)));
    assert_eq!(repeats.pressed(window, KEY, true, ms(285)), Some(Duration::from_millis(285)));
    assert_eq!(repeats.release(window, KEY, ms(300)), Some(Duration::from_millis(300)));
    assert_eq!(repeats.pressed(window, KEY, true, ms(310)), None, "released");
    assert_eq!(repeats.release(window, KEY, ms(320)), None);
    repeats.pressed(window, KEY, false, ms(400));
    repeats.retire_window(window);
    assert_eq!(repeats.release(window, KEY, ms(500)), None, "a retired window's holds end");
}
