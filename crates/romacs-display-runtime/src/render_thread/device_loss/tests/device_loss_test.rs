use super::{CONSECUTIVE_SURFACE_LOST_THRESHOLD, DeviceLossDetector, Fault, MAX_FAILED_RECOVERIES, fault};
use std::sync::atomic::Ordering;

#[test]
fn take_drains_the_latch_once() {
    let mut detector = DeviceLossDetector::new();
    assert!(!detector.take());

    detector.mark_lost_now();
    assert!(detector.take());
    assert!(!detector.take());
}

#[test]
fn shared_flag_feeds_take() {
    let mut detector = DeviceLossDetector::new();
    let flag = detector.shared_flag();

    flag.store(true, Ordering::SeqCst);
    assert!(detector.take());
    assert!(!detector.take());
}

#[test]
fn streak_escalates_exactly_at_the_threshold() {
    let mut detector = DeviceLossDetector::new();
    for _ in 0..CONSECUTIVE_SURFACE_LOST_THRESHOLD - 1 {
        assert!(!detector.record_surface_lost());
    }
    assert!(!detector.take(), "no latch below the threshold");

    assert!(
        detector.record_surface_lost(),
        "the Nth consecutive Lost escalates"
    );
    assert!(detector.take());

    // The streak restarted after escalation.
    assert!(!detector.record_surface_lost());
    assert!(!detector.take());
}

#[test]
fn successful_acquire_resets_the_streak() {
    let mut detector = DeviceLossDetector::new();
    for _ in 0..CONSECUTIVE_SURFACE_LOST_THRESHOLD - 1 {
        assert!(!detector.record_surface_lost());
    }
    detector.record_surface_acquired();

    for _ in 0..CONSECUTIVE_SURFACE_LOST_THRESHOLD - 1 {
        assert!(!detector.record_surface_lost());
    }
    assert!(!detector.take(), "acquired reset must restart the streak");
    assert!(detector.record_surface_lost());
    assert!(detector.take());
}

#[test]
fn taking_a_callback_loss_resets_the_streak() {
    let mut detector = DeviceLossDetector::new();
    for _ in 0..CONSECUTIVE_SURFACE_LOST_THRESHOLD - 1 {
        assert!(!detector.record_surface_lost());
    }
    detector.mark_lost_now();
    assert!(detector.take());
    // The rebuild starts from a clean streak.
    assert!(!detector.record_surface_lost());
}

fn validation(description: &str) -> wgpu::Error {
    wgpu::Error::Validation { source: description.into(), description: description.into() }
}

#[test]
fn a_validation_error_repaints_once_and_never_loses_the_device() {
    let mut detector = DeviceLossDetector::new();
    let handler = detector.uncaptured_errors();
    assert_eq!(fault(&validation("bad bind group")), Fault::Rejected("bad bind group".into()));
    handler(validation("bad bind group"));
    assert!(detector.take_repaint(), "a new error repaints");
    handler(validation("bad bind group"));
    assert!(!detector.take_repaint(), "the same error again is not told or repainted");
    handler(validation("bad buffer size"));
    assert!(detector.take_repaint(), "a different one is");
    assert!(!detector.take(), "no validation error is a device loss");
}

#[test]
fn out_of_memory_and_internal_errors_rebuild_the_device() {
    let mut detector = DeviceLossDetector::new();
    let handler = detector.uncaptured_errors();
    handler(wgpu::Error::OutOfMemory { source: "oom".into() });
    assert!(detector.take());
    handler(wgpu::Error::Internal { source: "internal".into(), description: "internal".into() });
    assert!(detector.take());
    assert!(!detector.take_repaint());
}

#[test]
fn only_consecutive_failed_rebuilds_give_up() {
    let mut detector = DeviceLossDetector::new();
    let now = romacs_display_protocol::frame_time::observe_platform_now();
    for _ in 0..MAX_FAILED_RECOVERIES - 1 {
        assert!(!detector.record_recovery(false, now));
    }
    assert!(!detector.record_recovery(true, now), "a rebuild that works resets the count");
    for _ in 0..MAX_FAILED_RECOVERIES - 1 {
        assert!(!detector.record_recovery(false, now));
    }
    assert!(detector.record_recovery(false, now), "the last failure in a row gives up");
}

/// Another wake (input, a resize) is no retry: a failed rebuild holds the
/// latched loss a second, so wakes cannot spend every attempt at once.
#[test]
fn a_failed_rebuild_holds_the_next_a_second() {
    let mut detector = DeviceLossDetector::new();
    let now = romacs_display_protocol::frame_time::observe_platform_now();
    detector.mark_lost_now();
    assert!(detector.take());
    detector.record_recovery(false, now);
    detector.mark_lost_now(); // what a failed recover_from_device_loss leaves
    let second = std::time::Duration::from_secs(1);
    assert_eq!(detector.retry_wait(now), Some(now.plus(second)));
    assert_eq!(detector.retry_wait(now.plus(second)), None, "due a second later");
    detector.record_recovery(true, now.plus(second));
    assert_eq!(detector.retry_wait(now), None, "a rebuild that works clears it");
}

#[test]
fn validation_errors_past_the_bound_are_not_told() {
    let detector = DeviceLossDetector::new();
    let handler = detector.uncaptured_errors();
    for n in 0..32 {
        handler(validation(&format!("error {n}")));
    }
    assert!(detector.take_repaint());
    handler(validation("error 33"));
    assert!(!detector.take_repaint(), "a full set stops telling and repainting");
}
