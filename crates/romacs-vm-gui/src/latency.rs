use std::time::Instant;

#[derive(Default)]
pub struct Trace {
    next: u64,
    pending: Vec<Key>,
    in_flight: Option<(u64, Vec<Key>, Paint, Instant, u128)>,
}

pub struct Key {
    pub sequence: u64,
    pub received: Option<Instant>,
    pub start: Instant,
    pub ready: Instant,
    pub command_ns: u128,
    pub diagnostic_ns: u128,
    pub reply_ns: u128,
}

pub struct Paint {
    pub submitted: Instant,
    pub layout_ns: u128,
    pub diagnostic_ns: u128,
    pub submit_ns: u128,
    pub layout_apply_ns: u128,
}

impl Trace {
    pub fn sequence(&mut self) -> u64 {
        self.next = self.next.checked_add(1).expect("key sequence exhausted");
        self.next
    }

    pub fn push(&mut self, key: Key) { self.pending.push(key); }

    pub fn submitted(&mut self, presentation: u64, paint: Paint, snapshot_start: Instant, snapshot_ns: u128) {
        assert!(self.in_flight.is_none());
        self.in_flight = Some((presentation, std::mem::take(&mut self.pending), paint, snapshot_start, snapshot_ns));
    }

    /// The keys since the last submission changed nothing shown, so no paint
    /// will present them: report each as complete, with no render time.
    pub fn unchanged(&mut self) -> Vec<serde_json::Value> {
        std::mem::take(&mut self.pending).into_iter().map(|key| {
            let origin = key.received.unwrap_or(key.start);
            serde_json::json!({"diagnostic":"latency-key","key_sequence":key.sequence,"presentation":null,
                "unchanged":true,"units":"ns","command_ns":key.command_ns,
                "key_to_command_complete_ns":key.start.duration_since(origin).as_nanos()+key.command_ns,
                "key_to_render_observed_ns":null})
        }).collect()
    }

    pub fn completed(&mut self, presentation: u64, observed: Instant) -> Vec<serde_json::Value> {
        if !self.in_flight.as_ref().is_some_and(|batch| batch.0 == presentation) { return Vec::new(); }
        let (_, keys, paint, snapshot_start, snapshot_ns) = self.in_flight.take().unwrap();
        keys.into_iter().map(|key| {
            let origin = key.received.unwrap_or(key.start);
            let queue_ns = key.start.duration_since(origin).as_nanos();
            let scheduler_ns = snapshot_start.duration_since(key.ready).as_nanos();
            let total_ns = observed.duration_since(origin).as_nanos();
            let accounted_ns = queue_ns + key.command_ns + key.diagnostic_ns + key.reply_ns + scheduler_ns
                + snapshot_ns + paint.layout_ns + paint.diagnostic_ns + paint.submit_ns + paint.layout_apply_ns;
            serde_json::json!({"diagnostic":"latency-key","key_sequence":key.sequence,"presentation":presentation,"units":"ns",
                "queue_ns":key.received.map(|_| queue_ns),"command_ns":key.command_ns,
                "diagnostic_ns":key.diagnostic_ns,"reply_ns":key.reply_ns,"scheduler_ns":scheduler_ns,
                "snapshot_ns":snapshot_ns,"layout_ns":paint.layout_ns,"paint_diagnostic_ns":paint.diagnostic_ns,
                "submit_ns":paint.submit_ns,"layout_apply_ns":paint.layout_apply_ns,
                "accounted_ns":accounted_ns,"residual_ns":total_ns.checked_sub(accounted_ns),
                "key_to_command_complete_ns":key.start.duration_since(origin).as_nanos()+key.command_ns,
                "key_to_render_observed_ns":key.received.map(|_| total_ns),
                "evaluator_to_render_observed_ns":observed.duration_since(key.start).as_nanos(),
                "submit_to_render_observed_ns":observed.duration_since(paint.submitted).as_nanos(),
                "completion_poll_interval_ns":1_000_000,
                "render_semantics":"renderer-successful-submit-observed-not-compositor-present",
                "observation_bias":"poll-and-evaluator-scheduling-overestimate"})
        }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn batches_map_each_key_to_next_submission_and_only_matching_completion() {
        let now = Instant::now();
        let mut trace = Trace::default();
        let key = |sequence| Key { sequence, received: Some(now), start: now, ready: now,
            command_ns: 2, diagnostic_ns: 3, reply_ns: 5 };
        let paint = || Paint { submitted: now, layout_ns: 7, diagnostic_ns: 11, submit_ns: 13, layout_apply_ns: 17 };
        let first = trace.sequence();
        trace.push(key(first));
        let second = trace.sequence();
        trace.push(key(second));
        trace.submitted(7, paint(), now, 0);
        let third = trace.sequence();
        trace.push(key(third));
        assert!(trace.completed(6, now).is_empty());
        let samples = trace.completed(7, now + Duration::from_nanos(100));
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0]["key_sequence"], 1);
        assert_eq!(samples[1]["key_sequence"], 2);
        assert_eq!(samples[0]["presentation"], 7);
        assert_eq!(samples[0]["accounted_ns"], 58);
        assert_eq!(samples[0]["residual_ns"], 42);
        assert!(trace.completed(7, now).is_empty());
        trace.submitted(8, paint(), now, 0);
        let samples = trace.completed(8, now);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0]["key_sequence"], 3);
    }

    #[test]
    fn keys_that_change_nothing_are_not_charged_to_a_later_paint() {
        let now = Instant::now();
        let mut trace = Trace::default();
        let key = |sequence| Key { sequence, received: Some(now), start: now, ready: now,
            command_ns: 2, diagnostic_ns: 0, reply_ns: 0 };
        let first = trace.sequence();
        trace.push(key(first));
        let unchanged = trace.unchanged();
        assert_eq!(unchanged.len(), 1);
        assert_eq!(unchanged[0]["unchanged"], true);
        assert!(unchanged[0]["key_to_render_observed_ns"].is_null());
        let second = trace.sequence();
        trace.push(key(second));
        trace.submitted(9, Paint { submitted: now, layout_ns: 0, diagnostic_ns: 0, submit_ns: 0, layout_apply_ns: 0 }, now, 0);
        let samples = trace.completed(9, now);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0]["key_sequence"], 2);
    }
}

/// AT on CLOCK_MONOTONIC, in nanoseconds: the clock a process outside the
/// editor stamps with, so a key's ingress can be matched with, say, its
/// byte's arrival at a terminal's pty (`key-ingress' records).
pub fn monotonic_ns(at: Instant) -> u128 {
    let now = rustix::time::clock_gettime(rustix::time::ClockId::Monotonic);
    (now.tv_sec as u128 * 1_000_000_000 + now.tv_nsec as u128).saturating_sub(at.elapsed().as_nanos())
}
