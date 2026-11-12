//! Frame-scheduling evidence counters (Stage 0 of the frame scheduling plan,
//! docs/plans/2026-07-11-cross-platform-frame-scheduling-and-animation-architecture.md).
//!
//! Process-wide monotonic counters incremented from the render thread's
//! scheduling and presentation hot points. They change no policy; they exist
//! so scheduling changes can be judged by structural evidence (wakeups,
//! requests, commits, glyph-build passes, submissions, commit-to-submission
//! latency) instead of by CPU percentages alone.
//!
//! Historical `present` counter/field names measure queue submission and
//! commit-to-submission latency, not compositor-confirmed display. Keep these
//! diagnostic names compatible; native presentation receipts are separate.
//!
//! All counters are relaxed atomics: every writer runs on the render thread,
//! and readers only need eventually-consistent totals.
//!
//! They are read three ways: [`snapshot`] from any thread, the file
//! `ROMACS_FRAME_STATS` names at the session's end, and, under
//! `ROMACS_DIAGNOSTICS=1`, a `frame-stats` line on stderr ([`StatsLines`]) a
//! second while frames are asked for or drawn and none at rest, which is how
//! a harness counts the frames of a state and the reasons they were drawn for
//! (`rest_live.py` among the GUI host's harnesses).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use romacs_display_protocol::frame_time::EventTime;

use super::frame_sched::{FramePlan, RenderWork};

/// One event-loop iteration (about_to_wait or user-event wake).
pub(super) static EVENT_LOOP_WAKEUPS: AtomicU64 = AtomicU64::new(0);
/// Redraw requests issued to winit windows.
pub(super) static REDRAW_REQUESTS: AtomicU64 = AtomicU64::new(0);
/// Redraw requests the loop issued because a scheduled deadline came due
/// rather than because a producer re-declared its demand. Proof that the
/// deadline-service step engages: demands with no per-pass producer (the
/// bounded Expose retry after a present produces nothing) reach the screen
/// only through this path, and before it existed their elapsed deadline was
/// re-armed as a zero-length wait on every pass -- a busy spin.
pub(super) static DEADLINE_SERVICED_REDRAWS: AtomicU64 = AtomicU64::new(0);
/// RedrawRequested events received from winit.
pub(super) static REDRAW_EVENTS: AtomicU64 = AtomicU64::new(0);
/// Editor scene commits ingested from the evaluator channel.
pub(super) static SCENE_COMMITS: AtomicU64 = AtomicU64::new(0);
/// Root glyph build/render passes (the expensive static-scene path).
pub(super) static ROOT_GLYPH_PASSES: AtomicU64 = AtomicU64::new(0);
/// Quads the root frame's pane chrome drew (VM2-PANE-CHROME): its dims,
/// corners and edges. Zero in GNU's look (`romacs-classic-chrome').
pub(super) static CHROME_DIVIDER_QUADS: AtomicU64 = AtomicU64::new(0);
/// Root glyph passes that drew the status bar's band (VM2-STATUS-BAR).
/// Zero without one (`romacs-classic-chrome').
pub(super) static CHROME_BAR_BANDS: AtomicU64 = AtomicU64::new(0);
/// Surface submissions on top-level frame windows.
pub(super) static SURFACE_SUBMISSIONS: AtomicU64 = AtomicU64::new(0);
/// Frame plans by render work class (frame scheduling plan, Stage 2).
pub(super) static PLAN_NONE: AtomicU64 = AtomicU64::new(0);
pub(super) static PLAN_COMPOSITE_ONLY: AtomicU64 = AtomicU64::new(0);
pub(super) static PLAN_REPAINT_LAYERS: AtomicU64 = AtomicU64::new(0);
pub(super) static PLAN_REBUILD_SCENE: AtomicU64 = AtomicU64::new(0);
/// Retained static-scene rebuilds — one per scene generation while the
/// cursor-only fast path is active (frame scheduling plan, Stage 4).
pub(super) static RETAINED_STATIC_BUILDS: AtomicU64 = AtomicU64::new(0);
/// Certified off-screen body rasterizations and viewport crops of those rasters.
pub(super) static SCROLL_RASTER_BUILDS: AtomicU64 = AtomicU64::new(0);
pub(super) static SCROLL_RASTER_BLITS: AtomicU64 = AtomicU64::new(0);
/// Frames served by the retained-static composite fast path (blit + cursor,
/// no glyph pipeline).
pub(super) static COMPOSITE_ONLY_FRAMES: AtomicU64 = AtomicU64::new(0);
/// Root glyph passes served by blitting the retained root picture.
pub(super) static RETAINED_ROOT_BLITS: AtomicU64 = AtomicU64::new(0);
/// Full-frame textures the GPU budget refused. Each one costs that frame its
/// offscreen composition (so its transitions and pane motion) or its post
/// shader, and the frame is still drawn. Structurally zero unless the render
/// thread is genuinely near its ceiling; a nonzero count is the evidence that
/// distinguishes "the animation is disabled" from "the animation could not be
/// afforded".
pub(super) static FULL_FRAME_TEXTURE_REFUSALS: AtomicU64 = AtomicU64::new(0);
/// Planned frames attributed per demand reason: one increment per reason a
/// planned frame satisfies, so an idle session's frames can be traced to what
/// asked for them. Indexed by [`super::frame_sched::DemandReason::index`]; a
/// frame driven by several reasons increments each.
static PLAN_DEMAND_REASONS: [AtomicU64; super::frame_sched::DemandReason::COUNT] =
    [const { AtomicU64::new(0) }; super::frame_sched::DemandReason::COUNT];
/// Frames that reached the surface, per demand reason: the same attribution
/// as [`PLAN_DEMAND_REASONS`], counted once the frame was submitted. A plan
/// that drew nothing (no surface, occluded, a timeout) counts above and not
/// here.
static PRESENTED_DEMAND_REASONS: [AtomicU64; super::frame_sched::DemandReason::COUNT] =
    [const { AtomicU64::new(0) }; super::frame_sched::DemandReason::COUNT];
/// Frames rendered without any demand reason to name. Structurally zero: the
/// redraw handler falls back to a platform-redraw plan when nothing it
/// scheduled explains the tick. Kept as standing evidence for architectural
/// invariant 12 ("every scheduled frame has at least one inspectable demand
/// reason"), which a counter can attest and a code reading cannot.
pub(super) static UNATTRIBUTED_SUBMISSION_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
/// Microseconds from the pending scene commit to surface submission.
pub(super) static LAST_COMMIT_TO_SUBMISSION_US: AtomicU64 = AtomicU64::new(0);
/// Worst observed commit-to-submission latency in microseconds.
pub(super) static MAX_COMMIT_TO_SUBMISSION_US: AtomicU64 = AtomicU64::new(0);

/// Commit-to-submission latency histogram: sample counts per log-scale microsecond
/// bucket (upper bounds in [`FRAME_TIME_BUCKET_UPPER_US`]). Lets a reader derive
/// frame-time percentiles (p50/p95/p99) rather than only last/max.
static FRAME_TIME_BUCKETS: [AtomicU64; 8] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

/// Inclusive upper bound (microseconds) of each `FRAME_TIME_BUCKETS` slot; the
/// last is unbounded (`u64::MAX`), i.e. frames slower than 66 ms.
pub(crate) const FRAME_TIME_BUCKET_UPPER_US: [u64; 8] =
    [1_000, 2_000, 4_000, 8_000, 16_000, 33_000, 66_000, u64::MAX];

fn record_frame_time(latency_us: u64) {
    let idx = FRAME_TIME_BUCKET_UPPER_US
        .iter()
        .position(|&upper| latency_us <= upper)
        .unwrap_or(FRAME_TIME_BUCKET_UPPER_US.len() - 1);
    FRAME_TIME_BUCKETS[idx].fetch_add(1, Ordering::Relaxed);
}

/// Monotonic anchor for converting observations to storable microsecond ticks.
static EPOCH: std::sync::OnceLock<EventTime> = std::sync::OnceLock::new();
/// Microsecond tick (vs EPOCH) of the oldest scene commit not yet submitted;
/// 0 = none pending. Approximate under multiple windows, which is acceptable
/// for Stage 0 evidence.
static PENDING_COMMIT_TICK_US: AtomicU64 = AtomicU64::new(0);

fn tick_us(now: EventTime) -> u64 {
    let epoch = *EPOCH.get_or_init(|| now);
    // Saturates at 0 for the first caller; monotonic afterwards. +1 keeps a
    // real tick from colliding with the 0 = "none pending" sentinel.
    now.saturating_since(epoch).as_micros() as u64 + 1
}

pub(super) fn count(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

/// Add N to COUNTER.
pub(super) fn add(counter: &AtomicU64, n: u64) {
    counter.fetch_add(n, Ordering::Relaxed);
}

/// Record a frame plan process-wide: its render work class and the demands
/// it satisfies.
pub(super) fn count_plan(plan: &FramePlan) {
    let counter = match plan.work {
        RenderWork::None => &PLAN_NONE,
        RenderWork::CompositeOnly { .. } => &PLAN_COMPOSITE_ONLY,
        RenderWork::RepaintLayers { .. } => &PLAN_REPAINT_LAYERS,
        RenderWork::RebuildScene => &PLAN_REBUILD_SCENE,
    };
    count(counter);
    for reason in plan.reasons.iter() {
        count(&PLAN_DEMAND_REASONS[reason.index()]);
    }
}

/// Record that PLAN's frame reached the surface: one increment per reason it
/// satisfies.
pub(super) fn count_presented(plan: &FramePlan) {
    for reason in plan.reasons.iter() {
        count(&PRESENTED_DEMAND_REASONS[reason.index()]);
    }
}

/// Record that a scene commit arrived; starts the commit-to-submission clock if
/// no earlier commit is still waiting for submission.
pub(super) fn note_scene_commit(now: EventTime) {
    count(&SCENE_COMMITS);
    let tick = tick_us(now);
    let _ = PENDING_COMMIT_TICK_US.compare_exchange(0, tick, Ordering::Relaxed, Ordering::Relaxed);
}

/// Record a top-level submission; closes the commit-to-submission measurement when
/// a commit is pending.
pub(super) fn note_submission(now: EventTime) {
    count(&SURFACE_SUBMISSIONS);
    let pending = PENDING_COMMIT_TICK_US.swap(0, Ordering::Relaxed);
    if pending != 0 {
        let latency_us = tick_us(now).saturating_sub(pending);
        LAST_COMMIT_TO_SUBMISSION_US.store(latency_us, Ordering::Relaxed);
        MAX_COMMIT_TO_SUBMISSION_US.fetch_max(latency_us, Ordering::Relaxed);
        record_frame_time(latency_us);
    }
}

/// Point-in-time copy of every counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameSchedSnapshot {
    pub wakeups: u64,
    pub redraw_requests: u64,
    /// Subset of `redraw_requests` issued by the event loop's deadline
    /// service (see [`DEADLINE_SERVICED_REDRAWS`]).
    pub deadline_serviced_redraws: u64,
    pub redraw_events: u64,
    pub scene_commits: u64,
    pub root_glyph_passes: u64,
    pub chrome_divider_quads: u64,
    pub chrome_bar_bands: u64,
    /// Surface submissions; historical field name retained for diagnostics.
    pub presents: u64,
    pub plan_none: u64,
    pub plan_composite_only: u64,
    pub plan_repaint_layers: u64,
    pub plan_rebuild_scene: u64,
    pub retained_static_builds: u64,
    pub scroll_raster_builds: u64,
    pub scroll_raster_blits: u64,
    pub composite_only_frames: u64,
    /// Latest commit-to-submission latency; historical diagnostic field name.
    pub last_commit_to_present_us: u64,
    /// Maximum commit-to-submission latency; historical diagnostic field name.
    pub max_commit_to_present_us: u64,
    /// Commit-to-submission latency histogram (counts per bucket; bounds in
    /// [`FRAME_TIME_BUCKET_UPPER_US`]).
    pub frame_time_buckets: [u64; 8],
    /// Planned frames per demand reason, indexed as [`DEMAND_REASON_NAMES`].
    pub demand_reasons: [u64; super::frame_sched::DemandReason::COUNT],
    /// Frames submitted to the surface per demand reason, indexed alike. The
    /// `platform_redraw` slot counts the redraws the coordinator did not ask
    /// for.
    pub presented_reasons: [u64; super::frame_sched::DemandReason::COUNT],
    /// Frames rendered with no demand reason. Must stay 0.
    pub unattributed_present_attempts: u64,
}

/// Names of the [`FrameSchedSnapshot::demand_reasons`] slots, in index order.
pub(crate) const DEMAND_REASON_NAMES: [&str; super::frame_sched::DemandReason::COUNT] = {
    let mut names = [""; super::frame_sched::DemandReason::COUNT];
    let all = super::frame_sched::DemandReason::ALL;
    let mut i = 0;
    while i < all.len() {
        names[all[i].index()] = all[i].name();
        i += 1;
    }
    names
};

pub fn snapshot() -> FrameSchedSnapshot {
    FrameSchedSnapshot {
        wakeups: EVENT_LOOP_WAKEUPS.load(Ordering::Relaxed),
        redraw_requests: REDRAW_REQUESTS.load(Ordering::Relaxed),
        deadline_serviced_redraws: DEADLINE_SERVICED_REDRAWS.load(Ordering::Relaxed),
        redraw_events: REDRAW_EVENTS.load(Ordering::Relaxed),
        scene_commits: SCENE_COMMITS.load(Ordering::Relaxed),
        root_glyph_passes: ROOT_GLYPH_PASSES.load(Ordering::Relaxed),
        chrome_divider_quads: CHROME_DIVIDER_QUADS.load(Ordering::Relaxed),
        chrome_bar_bands: CHROME_BAR_BANDS.load(Ordering::Relaxed),
        presents: SURFACE_SUBMISSIONS.load(Ordering::Relaxed),
        plan_none: PLAN_NONE.load(Ordering::Relaxed),
        plan_composite_only: PLAN_COMPOSITE_ONLY.load(Ordering::Relaxed),
        plan_repaint_layers: PLAN_REPAINT_LAYERS.load(Ordering::Relaxed),
        plan_rebuild_scene: PLAN_REBUILD_SCENE.load(Ordering::Relaxed),
        retained_static_builds: RETAINED_STATIC_BUILDS.load(Ordering::Relaxed),
        scroll_raster_builds: SCROLL_RASTER_BUILDS.load(Ordering::Relaxed),
        scroll_raster_blits: SCROLL_RASTER_BLITS.load(Ordering::Relaxed),
        composite_only_frames: COMPOSITE_ONLY_FRAMES.load(Ordering::Relaxed),
        last_commit_to_present_us: LAST_COMMIT_TO_SUBMISSION_US.load(Ordering::Relaxed),
        max_commit_to_present_us: MAX_COMMIT_TO_SUBMISSION_US.load(Ordering::Relaxed),
        frame_time_buckets: std::array::from_fn(|i| FRAME_TIME_BUCKETS[i].load(Ordering::Relaxed)),
        demand_reasons: std::array::from_fn(|i| PLAN_DEMAND_REASONS[i].load(Ordering::Relaxed)),
        presented_reasons: std::array::from_fn(|i| {
            PRESENTED_DEMAND_REASONS[i].load(Ordering::Relaxed)
        }),
        unattributed_present_attempts: UNATTRIBUTED_SUBMISSION_ATTEMPTS.load(Ordering::Relaxed),
    }
}

/// The least time between two diagnostic lines.
const LINE_INTERVAL: Duration = Duration::from_secs(1);

/// Redraws asked for and delivered, scenes ingested and frames submitted.
type FrameWork = [u64; 4];

/// The frame work SNAP counts: what a line reports as changed. Loop wakeups
/// are left out, or the wake that writes a line would ask for the next one.
fn frame_work(snap: &FrameSchedSnapshot) -> FrameWork {
    [
        snap.redraw_requests,
        snap.redraw_events,
        snap.scene_commits,
        snap.presents,
    ]
}

/// When the diagnostic lines were last written, and the frame work they
/// reported.
#[derive(Debug, Default)]
pub(super) struct StatsLines {
    last: Option<(EventTime, FrameWork)>,
}

/// What a pass of the loop does about the diagnostic line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineDue {
    /// Nothing to say: no frame work since the last line.
    Quiet,
    /// Write one now.
    Now,
    /// Frame work since the last line, written too recently: wake then.
    At(EventTime),
}

impl StatsLines {
    /// What is due at NOW with the frame WORK counted so far; a line due now
    /// is taken as written. The lines are a series of the counters at
    /// [`LINE_INTERVAL`]'s resolution whose last line holds the totals once
    /// the loop is quiet: a change is written at once, or at the interval's
    /// end when one was just written, and no change writes nothing, so a
    /// loop at rest is not woken.
    fn step(&mut self, now: EventTime, work: FrameWork) -> LineDue {
        match self.last {
            Some((_, reported)) if reported == work => LineDue::Quiet,
            Some((at, _)) if now < at.plus(LINE_INTERVAL) => LineDue::At(at.plus(LINE_INTERVAL)),
            _ => {
                self.last = Some((now, work));
                LineDue::Now
            }
        }
    }

    /// Under `ROMACS_DIAGNOSTICS=1`, write the counters to stderr as one
    /// `frame-stats` line when one is due at NOW; when the loop must wake to
    /// write the next. Called as the loop goes to wait.
    pub(super) fn service(&mut self, now: EventTime) -> Option<EventTime> {
        if !super::diagnostics() {
            return None;
        }
        let snap = snapshot();
        match self.step(now, frame_work(&snap)) {
            LineDue::Quiet => None,
            LineDue::At(at) => Some(at),
            LineDue::Now => {
                write_line(&line(&snap, tick_us(now)));
                None
            }
        }
    }
}

/// SNAP as a `frame-stats` diagnostic: the loop's counters and, per demand
/// reason that has any, the frames planned and presented for it.
fn line(snap: &FrameSchedSnapshot, at_us: u64) -> serde_json::Value {
    let by_reason = |counts: &[u64]| -> serde_json::Map<String, serde_json::Value> {
        DEMAND_REASON_NAMES
            .iter()
            .zip(counts)
            .filter(|(_, count)| **count > 0)
            .map(|(name, count)| ((*name).to_owned(), (*count).into()))
            .collect()
    };
    serde_json::json!({
        "diagnostic": "frame-stats",
        "at_us": at_us,
        "wakeups": snap.wakeups,
        "redraw_requests": snap.redraw_requests,
        "deadline_serviced_redraws": snap.deadline_serviced_redraws,
        "redraw_events": snap.redraw_events,
        "scene_commits": snap.scene_commits,
        "presents": snap.presents,
        "plan_none": snap.plan_none,
        "plan_composite_only": snap.plan_composite_only,
        "plan_repaint_layers": snap.plan_repaint_layers,
        "plan_rebuild_scene": snap.plan_rebuild_scene,
        "root_glyph_passes": snap.root_glyph_passes,
        "composite_only_frames": snap.composite_only_frames,
        "full_frame_texture_refusals": FULL_FRAME_TEXTURE_REFUSALS.load(Ordering::Relaxed),
        "unattributed_present_attempts": snap.unattributed_present_attempts,
        "planned": by_reason(&snap.demand_reasons),
        "presented": by_reason(&snap.presented_reasons),
    })
}

/// VALUE on stderr as one line, in one write.
fn write_line(value: &serde_json::Value) {
    use std::io::Write;
    let mut line = serde_json::to_vec(value).unwrap_or_default();
    line.push(b'\n');
    let _ = std::io::stderr().lock().write_all(&line);
}

#[cfg(test)]
#[path = "tests/frame_stats_test.rs"]
mod frame_stats_test;
