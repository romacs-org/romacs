//! `latency-key` records: one for each settled key, written as it settles,
//! with the loop's stage timings where it has them.
use super::*;
use romacs_vm::Unattributed;
use std::time::Duration;

fn at(instant: Instant) -> EventTime {
    EventTime::from_observed_instant(instant)
}

fn outcome(ingress: EventTime, outcome: Outcome) -> KeyOutcome {
    KeyOutcome { ingress, outcome }
}

fn frame(ingress: EventTime, presentation: u64) -> SubmittedFrame {
    SubmittedFrame { presentation, submitted_at: ingress.plus(Duration::from_nanos(30)) }
}

#[test]
fn a_key_the_loop_delivered_and_painted_has_its_stages() {
    let now = Instant::now();
    let mut trace = Trace::default();
    trace.delivered(at(now), Stages { start: now, ready: now, command_ns: 2, diagnostic_ns: 3, reply_ns: 5 });
    trace.submitted(7, Paint { layout_ns: 7, diagnostic_ns: 11, submit_ns: 13, layout_apply_ns: 17 }, now, 0);
    let samples = trace.settled(vec![outcome(at(now), Outcome::Rendered(frame(at(now), 7)))]);
    let sample = &samples[0];
    assert_eq!((&sample["key_sequence"], &sample["presentation"], &sample["outcome"]), (&json!(1), &json!(7), &json!("rendered")));
    assert_eq!((&sample["accounted_ns"], &sample["key_to_first_frame_ns"], &sample["command_ns"]), (&json!(58), &json!(30), &json!(2)));
}

/// A key read inside a command (a prompt's) has no stage timings of the
/// loop's; an opener is charged the frame it caused.
#[test]
fn a_prompts_keys_have_their_frames_alone() {
    let now = Instant::now();
    let mut trace = Trace::default();
    let samples = trace.settled(vec![outcome(at(now), Outcome::Opened(frame(at(now), 8)))]);
    assert_eq!((&samples[0]["outcome"], &samples[0]["presentation"], &samples[0]["key_to_first_frame_ns"]), (&json!("opened"), &json!(8), &json!(30)));
    assert!(samples[0]["command_ns"].is_null() && samples[0]["layout_ns"].is_null());
}

#[test]
fn every_settled_key_has_one_record_in_order() {
    let now = Instant::now();
    let mut trace = Trace::default();
    let keys: Vec<_> = (0..4).map(|i| at(now + Duration::from_nanos(i))).collect();
    trace.delivered(keys[1], Stages { start: now, ready: now, command_ns: 2, diagnostic_ns: 0, reply_ns: 0 });
    let samples = trace.settled(vec![
        outcome(keys[0], Outcome::Unattributed(Unattributed::AwaitedInput)),
        outcome(keys[1], Outcome::Unchanged),
        outcome(keys[2], Outcome::Rendered(frame(keys[2], 3))),
        outcome(keys[3], Outcome::Unattributed(Unattributed::OutsideCommand)),
    ]);
    assert_eq!(samples.iter().map(|s| (s["key_sequence"].clone(), s["outcome"].clone(), s["unattributed"].clone())).collect::<Vec<_>>(),
               vec![(json!(1), json!("unattributed"), json!("awaited-input")), (json!(2), json!("unchanged"), Value::Null),
                    (json!(3), json!("rendered"), Value::Null), (json!(4), json!("unattributed"), json!("outside-command"))]);
    assert_eq!(samples[1]["unchanged"], true);
    assert_eq!(samples[1]["key_to_command_complete_ns"], 2);
    assert!(samples[0]["key_to_first_frame_ns"].is_null() && samples[0]["presentation"].is_null());
}
