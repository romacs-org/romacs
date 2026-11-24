//! `--trace-latency`'s `latency-key` records: one for each native key the
//! evaluator settled (romacs-vm typing.rs `KeyOutcome`), written as it
//! settles: rendered or opened with the frame it caused, unchanged, or
//! unattributed with a reason. The loop adds the stage timings it measured
//! for a key it delivered and for a paint it submitted; a key read inside a
//! command (a prompt's) has none.
//!
//! A record holds no drawn time. The render thread traces when each
//! presentation drew (romacs-display-runtime `paint_host::trace_drawn`), and
//! a reader joins a key to the first drawn presentation at or after its
//! own: `ingress_ns` and that record's `drawn_ns` count from the same start
//! (`paint_host::trace_origin`).
use romacs_display_protocol::frame_time::EventTime;
use romacs_vm::{KeyOutcome, Outcome, SubmittedFrame};
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
    pub layout_ns: u128,
    pub diagnostic_ns: u128,
    pub submit_ns: u128,
    pub layout_apply_ns: u128,
}

/// The loop's keys and paints kept to time a key's record: a few, as a
/// key's outcome settles within a pass or two of both.
const KEPT: usize = 16;

#[derive(Default)]
pub struct Trace {
    next: u64,
    /// Keys the loop delivered, by ingress, until settled; one a prompt
    /// settled before its command returned has none.
    stages: Vec<(EventTime, Stages)>,
    /// The loop's paints by presentation, with their snapshot's start and
    /// length.
    paints: Vec<(u64, Paint, Instant, u128)>,
}

impl Trace {
    fn sequence(&mut self) -> u64 {
        self.next = self.next.checked_add(1).expect("key sequence exhausted");
        self.next
    }

    /// The loop delivered the key that arrived at INGRESS, as STAGES timed it.
    pub fn delivered(&mut self, ingress: EventTime, stages: Stages) {
        keep(&mut self.stages, (ingress, stages));
    }

    /// The loop submitted PRESENTATION, as PAINT timed it, its snapshot taken
    /// from SNAPSHOT_START for SNAPSHOT_NS.
    pub fn submitted(&mut self, presentation: u64, paint: Paint, snapshot_start: Instant, snapshot_ns: u128) {
        keep(&mut self.paints, (presentation, paint, snapshot_start, snapshot_ns));
    }

    /// The records of the keys OUTCOMES settled, in order.
    pub fn settled(&mut self, outcomes: Vec<KeyOutcome>) -> Vec<Value> {
        outcomes.into_iter().map(|KeyOutcome { ingress, outcome }| {
            let sequence = self.sequence();
            let stages = self.stages.iter().position(|(at, _)| *at == ingress).map(|at| self.stages.remove(at).1);
            self.record(sequence, ingress, &outcome, stages)
        }).collect()
    }

    fn record(&self, sequence: u64, ingress: EventTime, outcome: &Outcome, stages: Option<Stages>) -> Value {
        let ingress_ns = romacs_display_runtime::paint_host::trace_origin()
            .map(|origin| ingress.into_instant().saturating_duration_since(origin).as_nanos());
        let mut record = json!({"diagnostic":"latency-key","key_sequence":sequence,"units":"ns","ingress_ns":ingress_ns,
            "presentation":null,"key_to_first_frame_ns":null});
        merge(&mut record, outcome_fields(outcome));
        let origin = ingress.into_instant();
        if let Some(stages) = stages { merge(&mut record, stage_fields(origin, stages)); }
        if let Outcome::Rendered(SubmittedFrame { presentation, submitted_at }) | Outcome::Opened(SubmittedFrame { presentation, submitted_at }) = *outcome {
            merge(&mut record, json!({"presentation":presentation,
                "key_to_first_frame_ns":submitted_at.saturating_since(ingress).as_nanos()}));
            if let Some(&(_, paint, snapshot_start, snapshot_ns)) = self.paints.iter().find(|(p, ..)| *p == presentation) {
                merge(&mut record, paint_fields(paint, snapshot_ns));
                if let Some(stages) = stages { merge(&mut record, accounted(origin, stages, (paint, snapshot_start, snapshot_ns))); }
            }
        }
        record
    }
}

/// ITEM kept among the last `KEPT` of ITEMS.
fn keep<T>(items: &mut Vec<T>, item: T) {
    if items.len() == KEPT { items.remove(0); }
    items.push(item);
}

/// OUTCOME's name, and `unchanged` or the reason a key was charged no frame.
fn outcome_fields(outcome: &Outcome) -> Value {
    match outcome {
        Outcome::Rendered(_) => json!({"outcome":"rendered"}),
        Outcome::Opened(_) => json!({"outcome":"opened"}),
        Outcome::Unchanged => json!({"outcome":"unchanged","unchanged":true}),
        Outcome::Unattributed(why) => json!({"outcome":"unattributed","unattributed":why.name()}),
    }
}

fn stage_fields(origin: Instant, stages: Stages) -> Value {
    let queue_ns = stages.start.saturating_duration_since(origin).as_nanos();
    json!({"queue_ns":queue_ns,"command_ns":stages.command_ns,"diagnostic_ns":stages.diagnostic_ns,"reply_ns":stages.reply_ns,
        "key_to_command_complete_ns":queue_ns + stages.command_ns})
}

fn paint_fields(paint: Paint, snapshot_ns: u128) -> Value {
    json!({"snapshot_ns":snapshot_ns,"layout_ns":paint.layout_ns,"paint_diagnostic_ns":paint.diagnostic_ns,
        "submit_ns":paint.submit_ns,"layout_apply_ns":paint.layout_apply_ns})
}

/// The stages a key delivered and painted by the loop is accounted for by,
/// up to its paint's submission: a reader's drawn time less this is the
/// residual.
fn accounted(origin: Instant, stages: Stages, (paint, snapshot_start, snapshot_ns): (Paint, Instant, u128)) -> Value {
    let queue_ns = stages.start.saturating_duration_since(origin).as_nanos();
    let scheduler_ns = snapshot_start.saturating_duration_since(stages.ready).as_nanos();
    let accounted_ns = queue_ns + stages.command_ns + stages.diagnostic_ns + stages.reply_ns + scheduler_ns
        + snapshot_ns + paint.layout_ns + paint.diagnostic_ns + paint.submit_ns + paint.layout_apply_ns;
    json!({"scheduler_ns":scheduler_ns,"accounted_ns":accounted_ns})
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
