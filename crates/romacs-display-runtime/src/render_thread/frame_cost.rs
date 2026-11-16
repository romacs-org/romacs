//! What each drawn frame cost the render thread, said on stderr.
//!
//! The renderer counts its work on this thread as it does it
//! (`romacs_renderer_wgpu::cost`), and the stretches named here are metered
//! in instructions (`romacs_display_protocol::meter`) once the session
//! installed a counter, which the GUI host does under
//! `ROMACS_DIAGNOSTICS=1`. Every frame drawn from then on writes one line,
//!
//!   {"diagnostic":"frame-cost","presentation":N,"work":W,"result":R,
//!    "scenes":S,COUNT:n,...}
//!
//! where each COUNT is what the thread added to it since the line before:
//! the frame's own draw and what led up to it, its scene's ingestion above
//! all. W is the plan's work class, R how the draw ended, S the scenes
//! ingested since the line before. Of the instruction counts,
//! `frame_instructions` is the whole draw, and inside it
//! `acquire_instructions`, `submit_instructions` and `present_instructions`
//! are the surface's and the driver's, so the frame less those three is
//! this code's own; `ingest_instructions` is beside the frame, and its own
//! too. `render_stages_live.py` among the GUI host's harnesses holds the
//! figures to bars by scenario.
//!
//! Without a counter nothing is metered, nothing is written, and a frame
//! pays the renderer's plain adds alone.
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::Ordering;

use romacs_display_protocol::{call_count, meter};

use super::frame_sched::SubmissionResult;

/// A scene taken from the evaluator's channel into the window's state.
pub(super) const INGEST: &str = "render.ingest-instructions";
/// A frame's draw, from its plan to the surface's present.
pub(super) const FRAME: &str = "render.frame-instructions";
/// Inside the draw: the surface's next texture acquired.
pub(super) const ACQUIRE: &str = "render.acquire-instructions";
/// Inside the draw: the surface presented.
pub(super) const PRESENT: &str = "render.present-instructions";

/// Before the draw: the page scenes' and shader surfaces' own frames
/// (a text animation's scene), which change with no new frame.
pub(super) const SCENES: &str = "render.scenes-instructions";

const OWN: [&str; 5] = [INGEST, FRAME, ACQUIRE, PRESENT, SCENES];

thread_local! {
    /// The thread's counts as the last line read them, and the scenes
    /// ingested by then.
    static SAID: RefCell<(HashMap<&'static str, u64>, u64)> = RefCell::new((HashMap::new(), 0));
}

/// What a line reports: this module's stretches, then the renderer's counts.
fn names() -> impl Iterator<Item = &'static str> {
    OWN.into_iter().chain(romacs_renderer_wgpu::cost::COUNTS)
}

/// A count's key in the line: its name less the `render.` all share, in
/// JSON's spelling.
fn key(name: &str) -> String {
    name.trim_start_matches("render.").replace('-', "_")
}

/// The line of a frame of PRESENTATION drawn as WORK with RESULT, after
/// SCENES more scenes and COUNTS more of each count.
fn line(presentation: u64, work: &str, result: SubmissionResult, scenes: u64, counts: &[(&str, u64)]) -> serde_json::Value {
    let mut said = serde_json::json!({
        "diagnostic": "frame-cost",
        "presentation": presentation,
        "work": work,
        "result": format!("{result:?}"),
        "scenes": scenes,
    });
    for (name, count) in counts {
        said[key(name)] = (*count).into();
    }
    said
}

/// What NOW adds to each count over LAST, the counts of the line before.
fn added(now: &[(&'static str, u64)], last: &HashMap<&'static str, u64>) -> Vec<(&'static str, u64)> {
    now.iter()
        .map(|&(name, count)| (name, count.saturating_sub(last.get(name).copied().unwrap_or(0))))
        .collect()
}

/// Say the cost of the frame of PRESENTATION just drawn as WORK with
/// RESULT, when the session meters.
pub(super) fn say(presentation: u64, work: &str, result: SubmissionResult) {
    if !meter::installed() {
        return;
    }
    let now: Vec<_> = names().map(|name| (name, call_count::of(name))).collect();
    let scenes = super::frame_stats::SCENE_COMMITS.load(Ordering::Relaxed);
    let said = SAID.with_borrow_mut(|(last, last_scenes)| {
        let said = line(presentation, work, result, scenes.saturating_sub(*last_scenes), &added(&now, last));
        last.extend(now.iter().copied());
        *last_scenes = scenes;
        said
    });
    write_line(&said);
}

/// VALUE on stderr as one line, in one write.
fn write_line(value: &serde_json::Value) {
    use std::io::Write;
    let mut line = serde_json::to_vec(value).unwrap_or_default();
    line.push(b'\n');
    let _ = std::io::stderr().lock().write_all(&line);
}

#[cfg(test)]
#[path = "tests/frame_cost_test.rs"]
mod frame_cost_test;
