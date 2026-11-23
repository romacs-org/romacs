//! `--trace-latency`'s `latency-key` records: one for each native key the
//! evaluator settled (romacs-vm typing.rs `KeyOutcome`), rendered,
//! unchanged, or unattributed with a reason. A rendered key's record waits
//! for its presentation, or a later one, to be seen drawn. The loop adds the
//! stage timings it measured for the keys it delivered and the paints it
//! submitted; a key read inside a command (a prompt's) has none.
use romacs_display_protocol::frame_time::EventTime;
use romacs_display_protocol::submission::SubmissionReceipt;
use romacs_vm::{KeyOutcome, Outcome};
use romacs_vm_gui::presentation::Drawn;
use serde_json::{Value, json};
use std::time::Instant;

/// The loop's timings of a key it delivered, from its command's start.
#[derive(Clone, Copy)]
pub struct Stages {
    pub start: Instant,
    pub ready: Instant,
    pub command_ns: u128,
    pub diagnostic_ns: u128,
    pub reply_ns: u128,
}

/// The loop's timings of a paint it submitted.
#[derive(Clone, Copy)]
pub struct Paint {
    pub submitted: Instant,
    pub layout_ns: u128,
    pub diagnostic_ns: u128,
    pub submit_ns: u128,
    pub layout_apply_ns: u128,
}

/// A rendered key, until its frame is seen drawn.
struct Rendered {
    sequence: u64,
    ingress: EventTime,
    receipt: SubmissionReceipt,
    stages: Option<Stages>,
}

#[derive(Default)]
pub struct Trace {
    next: u64,
    /// Keys the loop delivered, by ingress, until settled.
    stages: Vec<(EventTime, Stages)>,
    /// The loop's paints by presentation, with their snapshot's start and
    /// length, until drawn.
    paints: Vec<(u64, Paint, Instant, u128)>,
    rendered: Vec<Rendered>,
    drawn: Option<Drawn>,
}

impl Trace {
    fn sequence(&mut self) -> u64 {
        self.next = self.next.checked_add(1).expect("key sequence exhausted");
        self.next
    }

    /// The loop delivered the key that arrived at INGRESS, as STAGES timed it.
    pub fn delivered(&mut self, ingress: EventTime, stages: Stages) {
        self.stages.push((ingress, stages));
    }

    /// The loop submitted PRESENTATION, as PAINT timed it, its snapshot taken
    /// from SNAPSHOT_START for SNAPSHOT_NS.
    pub fn submitted(&mut self, presentation: u64, paint: Paint, snapshot_start: Instant, snapshot_ns: u128) {
        self.paints.push((presentation, paint, snapshot_start, snapshot_ns));
    }

    /// The records of the keys OUTCOMES settled, but for those rendered in a
    /// frame not yet seen drawn, which `seen_drawn` tells.
    pub fn settled(&mut self, outcomes: Vec<KeyOutcome>) -> Vec<Value> {
        let mut records = Vec::new();
        for KeyOutcome { ingress, outcome } in outcomes {
            let sequence = self.sequence();
            let stages = self.stages.iter().position(|(at, _)| *at == ingress).map(|at| self.stages.remove(at).1);
            match outcome {
                Outcome::Rendered(receipt) => self.rendered.push(Rendered { sequence, ingress, receipt, stages }),
                Outcome::Unchanged => records.push(unrendered(sequence, ingress, stages, json!({"unchanged":true}))),
                Outcome::Unattributed(why) => records.push(unrendered(sequence, ingress, stages, json!({"unattributed":why.name()}))),
            }
        }
        records.extend(self.drawn.map(|drawn| self.seen_drawn(drawn)).unwrap_or_default());
        records
    }

    /// DRAWN was seen drawn: the records of the keys rendered in it or in an
    /// earlier presentation.
    pub fn seen_drawn(&mut self, drawn: Drawn) -> Vec<Value> {
        self.drawn = Some(drawn);
        let (shown, waiting) = std::mem::take(&mut self.rendered).into_iter().partition(|key| key.receipt.presentation <= drawn.presentation);
        self.rendered = waiting;
        let records = shown.into_iter().map(|key| self.rendered_record(key, drawn)).collect();
        self.paints.retain(|(presentation, ..)| *presentation > drawn.presentation);
        records
    }

    fn rendered_record(&self, mut key: Rendered, drawn: Drawn) -> Value {
        key.receipt.drawn_at = drawn.drawn_at;
        let origin = key.ingress.into_instant();
        let total_ns = drawn.observed.saturating_duration_since(origin).as_nanos();
        let mut record = json!({"diagnostic":"latency-key","key_sequence":key.sequence,"presentation":key.receipt.presentation,
            "outcome":"rendered","units":"ns","key_to_render_observed_ns":total_ns,
            "key_to_first_frame_ns":key.receipt.submitted_at.saturating_since(key.ingress).as_nanos(),
            "key_to_drawn_ns":key.receipt.drawn_at.map(|at| at.saturating_since(key.ingress).as_nanos()),
            "completion_poll_interval_ns":1_000_000,
            "render_semantics":"renderer-successful-submit-observed-not-compositor-present",
            "observation_bias":"poll-and-evaluator-scheduling-overestimate"});
        let paint = self.paints.iter().find(|(presentation, ..)| *presentation == key.receipt.presentation);
        if let Some(stages) = key.stages {
            merge(&mut record, stage_fields(origin, stages));
            merge(&mut record, json!({"evaluator_to_render_observed_ns":drawn.observed.saturating_duration_since(stages.start).as_nanos()}));
        }
        if let Some(&(_, paint, snapshot_start, snapshot_ns)) = paint {
            merge(&mut record, paint_fields(paint, snapshot_ns, drawn.observed));
            if let Some(stages) = key.stages {
                merge(&mut record, accounted(origin, stages, (paint, snapshot_start, snapshot_ns), total_ns));
            }
        }
        record
    }
}

/// A key's record with no frame: OUTCOME says why.
fn unrendered(sequence: u64, ingress: EventTime, stages: Option<Stages>, outcome: Value) -> Value {
    let mut record = json!({"diagnostic":"latency-key","key_sequence":sequence,"presentation":null,"units":"ns",
        "key_to_render_observed_ns":null,"key_to_first_frame_ns":null,"key_to_drawn_ns":null});
    merge(&mut record, outcome);
    if let Some(stages) = stages { merge(&mut record, stage_fields(ingress.into_instant(), stages)); }
    record
}

fn stage_fields(origin: Instant, stages: Stages) -> Value {
    let queue_ns = stages.start.saturating_duration_since(origin).as_nanos();
    json!({"queue_ns":queue_ns,"command_ns":stages.command_ns,"diagnostic_ns":stages.diagnostic_ns,"reply_ns":stages.reply_ns,
        "key_to_command_complete_ns":queue_ns + stages.command_ns})
}

fn paint_fields(paint: Paint, snapshot_ns: u128, observed: Instant) -> Value {
    json!({"snapshot_ns":snapshot_ns,"layout_ns":paint.layout_ns,"paint_diagnostic_ns":paint.diagnostic_ns,
        "submit_ns":paint.submit_ns,"layout_apply_ns":paint.layout_apply_ns,
        "submit_to_render_observed_ns":observed.saturating_duration_since(paint.submitted).as_nanos()})
}

/// The stages a key delivered and painted by the loop is accounted for, and
/// what of TOTAL_NS they leave.
fn accounted(origin: Instant, stages: Stages, (paint, snapshot_start, snapshot_ns): (Paint, Instant, u128), total_ns: u128) -> Value {
    let queue_ns = stages.start.saturating_duration_since(origin).as_nanos();
    let scheduler_ns = snapshot_start.saturating_duration_since(stages.ready).as_nanos();
    let accounted_ns = queue_ns + stages.command_ns + stages.diagnostic_ns + stages.reply_ns + scheduler_ns
        + snapshot_ns + paint.layout_ns + paint.diagnostic_ns + paint.submit_ns + paint.layout_apply_ns;
    json!({"scheduler_ns":scheduler_ns,"accounted_ns":accounted_ns,"residual_ns":total_ns.checked_sub(accounted_ns)})
}

fn merge(record: &mut Value, fields: Value) {
    if let (Value::Object(record), Value::Object(fields)) = (record, fields) { record.extend(fields); }
}

/// AT on CLOCK_MONOTONIC, in nanoseconds: the clock a process outside the
/// editor stamps with, so a key's ingress can be matched with, say, its
/// byte's arrival at a terminal's pty (`key-ingress' records).
pub fn monotonic_ns(at: Instant) -> u128 {
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    (now.tv_sec as u128 * 1_000_000_000 + now.tv_nsec as u128).saturating_sub(at.elapsed().as_nanos())
}

#[cfg(test)]
#[path = "latency_tests.rs"]
mod tests;
