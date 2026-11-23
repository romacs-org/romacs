//! `latency-key` records: one for each settled key, a rendered one once its
//! frame is seen drawn, with the loop's stage timings where it has them.
use super::*;
use romacs_vm::Unattributed;
use std::time::Duration;

fn at(instant: Instant) -> EventTime {
    EventTime::from_observed_instant(instant)
}

fn outcome(ingress: EventTime, outcome: Outcome) -> KeyOutcome {
    KeyOutcome { ingress, outcome }
}

fn rendered(ingress: EventTime, presentation: u64) -> KeyOutcome {
    let receipt = SubmissionReceipt { presentation, submitted_at: ingress.plus(Duration::from_nanos(30)), drawn_at: None, window: None };
    outcome(ingress, Outcome::Rendered(receipt))
}

fn drawn(presentation: u64, observed: Instant, drawn_at: Option<EventTime>) -> Drawn {
    Drawn { presentation, observed, drawn_at }
}

#[test]
fn a_rendered_key_waits_for_its_frame_to_draw() {
    let now = Instant::now();
    let mut trace = Trace::default();
    trace.delivered(at(now), Stages { start: now, ready: now, command_ns: 2, diagnostic_ns: 3, reply_ns: 5 });
    trace.submitted(7, Paint { submitted: now, layout_ns: 7, diagnostic_ns: 11, submit_ns: 13, layout_apply_ns: 17 }, now, 0);
    assert!(trace.settled(vec![rendered(at(now), 7)]).is_empty());
    assert!(trace.seen_drawn(drawn(6, now, None)).is_empty(), "an earlier frame does not show it");
    let samples = trace.seen_drawn(drawn(7, now + Duration::from_nanos(100), Some(at(now + Duration::from_nanos(90)))));
    assert_eq!(samples.len(), 1);
    let sample = &samples[0];
    assert_eq!((&sample["key_sequence"], &sample["presentation"], &sample["outcome"]), (&json!(1), &json!(7), &json!("rendered")));
    assert_eq!((&sample["accounted_ns"], &sample["residual_ns"]), (&json!(58), &json!(42)));
    assert_eq!((&sample["key_to_first_frame_ns"], &sample["key_to_drawn_ns"], &sample["key_to_render_observed_ns"]), (&json!(30), &json!(90), &json!(100)));
    assert!(trace.seen_drawn(drawn(7, now, None)).is_empty(), "one record a key");
}

/// A key read inside a command (a prompt's) has no stage timings of the
/// loop's; one rendered in a frame already seen drawn is told at once.
#[test]
fn a_key_rendered_in_a_frame_already_drawn_is_told_at_once() {
    let now = Instant::now();
    let mut trace = Trace::default();
    assert!(trace.seen_drawn(drawn(9, now, None)).is_empty());
    let samples = trace.settled(vec![rendered(at(now), 8)]);
    assert_eq!(samples.len(), 1);
    assert!(samples[0]["command_ns"].is_null() && samples[0]["layout_ns"].is_null());
    assert_eq!(samples[0]["key_to_first_frame_ns"], 30);
}

#[test]
fn every_settled_key_has_one_record_in_order() {
    let now = Instant::now();
    let mut trace = Trace::default();
    let keys: Vec<_> = (0..4).map(|i| at(now + Duration::from_nanos(i))).collect();
    trace.delivered(keys[1], Stages { start: now, ready: now, command_ns: 2, diagnostic_ns: 0, reply_ns: 0 });
    let samples = trace.settled(vec![
        outcome(keys[0], Outcome::Unattributed(Unattributed::Superseded)),
        outcome(keys[1], Outcome::Unchanged),
        rendered(keys[2], 3),
        outcome(keys[3], Outcome::Unattributed(Unattributed::OutsideCommand)),
    ]);
    assert_eq!(samples.iter().map(|s| (s["key_sequence"].clone(), s["unattributed"].clone(), s["unchanged"].clone())).collect::<Vec<_>>(),
               vec![(json!(1), json!("superseded"), Value::Null), (json!(2), Value::Null, json!(true)), (json!(4), json!("outside-command"), Value::Null)]);
    assert!(samples.iter().all(|s| s["key_to_render_observed_ns"].is_null() && s["key_to_first_frame_ns"].is_null()));
    assert_eq!(samples[1]["key_to_command_complete_ns"], 2);
    let samples = trace.seen_drawn(drawn(3, now, None));
    assert_eq!(samples.iter().map(|s| s["key_sequence"].clone()).collect::<Vec<_>>(), vec![json!(3)]);
}
