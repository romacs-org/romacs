use super::*;
use crate::render_thread::frame_sched::{
    ClockSource, DemandReason, DemandReasonSet, FrameTick, RenderWork,
};

fn at(start: EventTime, ms: u64) -> EventTime {
    start.plus(Duration::from_millis(ms))
}

#[test]
fn a_line_follows_frame_work_at_once_or_at_the_intervals_end_and_rest_writes_none() {
    let start = romacs_display_protocol::frame_time::observe_platform_now();
    let mut lines = StatsLines::default();
    assert_eq!(lines.due(start, [0; 4]), LineDue::Now, "the first pass says where the counters stand");
    lines.last = Some((start, [0; 4]));
    assert_eq!(lines.due(at(start, 5_000), [0; 4]), LineDue::Quiet, "at rest: no line, no wake");
    let drew = [1, 1, 0, 1];
    assert_eq!(
        lines.due(at(start, 300), drew),
        LineDue::At(at(start, 1_000)),
        "work just after a line: the loop wakes to write it"
    );
    assert_eq!(lines.due(at(start, 1_000), drew), LineDue::Now);
    lines.last = Some((at(start, 1_000), drew));
    assert_eq!(lines.due(at(start, 9_000), drew), LineDue::Quiet, "the last line holds the totals");
    assert_eq!(lines.due(at(start, 9_000), [2, 2, 0, 2]), LineDue::Now);
}

#[test]
fn a_presented_frame_counts_under_each_of_its_reasons() {
    let now = romacs_display_protocol::frame_time::observe_platform_now();
    let plan = FramePlan {
        tick: FrameTick {
            frame_time: now,
            target_presentation_time: now,
            estimated_interval: Duration::from_millis(16),
            source: ClockSource::Synthetic,
        },
        work: RenderWork::RebuildScene,
        should_present: true,
        reasons: [DemandReason::Redisplay, DemandReason::PlatformRedraw]
            .into_iter()
            .collect::<DemandReasonSet>(),
    };
    // The counters are the process's: other tests draw too, so they only grow.
    let before = snapshot();
    count_presented(&plan);
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
}
