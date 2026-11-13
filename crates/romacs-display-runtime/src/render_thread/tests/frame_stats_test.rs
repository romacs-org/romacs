use super::*;
use crate::render_thread::frame_sched::{
    ClockSource, DemandReason, DemandReasonSet, FrameTick, RenderWork,
};

fn at(start: EventTime, ms: u64) -> EventTime {
    start.plus(Duration::from_millis(ms))
}

/// A plan satisfying REASONS, as `draw_window` hands one to the count.
fn plan(reasons: &[DemandReason]) -> FramePlan {
    let now = romacs_display_protocol::frame_time::observe_platform_now();
    FramePlan {
        tick: FrameTick {
            frame_time: now,
            target_presentation_time: now,
            estimated_interval: Duration::from_millis(16),
            source: ClockSource::Synthetic,
        },
        work: RenderWork::RebuildScene,
        should_present: true,
        reasons: reasons.iter().copied().collect::<DemandReasonSet>(),
    }
}

#[test]
fn a_line_follows_frame_work_at_once_or_at_the_intervals_end_and_rest_writes_none() {
    let start = romacs_display_protocol::frame_time::observe_platform_now();
    let mut lines = StatsLines::default();
    assert_eq!(lines.step(start, [0; 4]), LineDue::Now, "the first pass says where the counters stand");
    assert_eq!(lines.step(at(start, 200), [0; 4]), LineDue::Quiet, "at rest: no line, no wake");
    let drew = [1, 1, 0, 1];
    assert_eq!(
        lines.step(at(start, 300), drew),
        LineDue::At(at(start, 1_000)),
        "work just after a line: the loop wakes to write it"
    );
    assert_eq!(lines.step(at(start, 999), drew), LineDue::At(at(start, 1_000)), "an early wake waits on");
    assert_eq!(lines.step(at(start, 1_000), drew), LineDue::Now);
    assert_eq!(lines.step(at(start, 9_000), drew), LineDue::Quiet, "the last line holds the totals");
    assert_eq!(lines.step(at(start, 9_000), [2, 2, 0, 2]), LineDue::Now, "a rest ends: said at once");
    assert_eq!(lines.step(at(start, 9_400), [3, 3, 0, 3]), LineDue::At(at(start, 10_000)));
}

#[test]
fn a_presented_frame_counts_under_each_of_its_reasons() {
    // The counters are the process's: other tests draw too, so they only grow.
    let before = snapshot();
    count_drawn(&plan(&[DemandReason::Redisplay, DemandReason::PlatformRedraw]), SubmissionResult::Submitted);
    let after = snapshot();
    for reason in [DemandReason::Redisplay, DemandReason::PlatformRedraw] {
        assert!(after.presented_reasons[reason.index()] > before.presented_reasons[reason.index()]);
    }
    let said = line(&after, 7);
    assert_eq!(said["diagnostic"], "frame-stats");
    assert_eq!(said["at_us"], 7);
    assert_eq!(
        said["presented"]["platform_redraw"],
        after.presented_reasons[DemandReason::PlatformRedraw.index()]
    );
    assert!(said["presented"].get("video").is_none(), "a reason with no frames is left out");
    assert_eq!(said["unattributed_present_attempts"], after.unattributed_present_attempts);
}

#[test]
fn a_frame_that_did_not_reach_the_surface_is_not_presented() {
    // No other test presents a debug capture, so its count is this test's own.
    let capture = plan(&[DemandReason::DebugCapture]);
    let presented = || snapshot().presented_reasons[DemandReason::DebugCapture.index()];
    let before = presented();
    for failed in [
        SubmissionResult::AwaitingContent,
        SubmissionResult::Skipped,
        SubmissionResult::Occluded,
        SubmissionResult::SurfaceLost,
        SubmissionResult::Timeout,
    ] {
        count_drawn(&capture, failed);
        assert_eq!(presented(), before, "{failed:?} drew nothing");
    }
    count_drawn(&capture, SubmissionResult::Submitted);
    assert_eq!(presented(), before + 1);
}
