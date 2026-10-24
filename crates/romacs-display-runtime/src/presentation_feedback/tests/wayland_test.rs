use super::*;
use romacs_display_protocol::PresentationId;

fn submission(serial: u64, layout: u64) -> Submission {
    Submission {
        serial,
        frame: 7,
        layout: PresentationId::new(layout),
        draw_seq: Some(serial),
        width: 800,
        height: 600,
        scale: 1.0,
    }
}

#[test]
fn trace_only_feedback_requires_explicit_enablement() {
    assert!(matches!(
        initial_state(None, || false),
        ObserverState::Disabled
    ));
    assert!(matches!(
        initial_state(None, || true),
        ObserverState::Uninitialized(None)
    ));
}

#[test]
fn receipt_feedback_does_not_depend_on_trace_enablement() {
    let path = PathBuf::from("/run/romacs-receipt");
    let state = initial_state(Some(path.clone()), || {
        panic!("receipt feedback must not depend on tracing")
    });
    assert!(matches!(state, ObserverState::Uninitialized(Some(actual)) if actual == path));
}

#[test]
fn trace_only_feedback_never_advances_file_receipts() {
    let mut receipts = Receipts {
        path: None,
        latest_presented: 0,
        clock_id: Some(1),
        pending: PendingReceipts::default(),
    };
    receipts.pending.requested(1, observe_platform_now());
    receipts.observe(NativeFeedback::Presented(ConfirmedPresentation {
        submission: submission(1, 100),
        timestamp: CompositorTimestamp {
            clock_id: 1,
            seconds: 42,
            nanoseconds: 123,
        },
    }));
    assert!(receipts.pending.0.is_empty());
    assert_eq!(receipts.latest_presented, 0);
}

#[test]
fn pending_receipts_wake_without_new_input_and_stop_after_ack_or_timeout() {
    let now = observe_platform_now();
    let mut pending = PendingReceipts::default();
    assert_eq!(pending.deadline(now), None);
    pending.requested(1, now);
    pending.requested(2, now);
    assert_eq!(
        pending.deadline(now),
        Some(now.plus(Duration::from_millis(4)))
    );
    pending.received(2);
    assert!(pending.deadline(now).is_some());
    pending.received(1);
    assert_eq!(pending.deadline(now), None);
    pending.requested(3, now);
    assert_eq!(pending.deadline(now.plus(Duration::from_secs(1))), None);
}

#[test]
fn reordered_native_feedback_preserves_draw_cookies() {
    use romacs_display_protocol::present_trace::{self, DrawScope, Stage};

    const CHILD: &str = "ROMACS_NATIVE_FEEDBACK_TRACE_CHILD";
    const NAME: &str =
        "presentation_feedback::wayland::tests::reordered_native_feedback_preserves_draw_cookies";
    if std::env::var_os(CHILD).is_none() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("trace.jsonl");
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", NAME])
            .env(CHILD, "1")
            .env("ROMACS_PRESENT_TRACE", &path)
            .env_remove("ROMACS_INPUT_LATENCY_FILE")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        let records: Vec<serde_json::Value> = fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let starts: Vec<_> = records
            .iter()
            .filter(|r| r["stage"] == "render_start")
            .collect();
        let feedback: Vec<_> = records
            .iter()
            .filter(|r| r["stage"] == "presented" || r["stage"] == "discarded")
            .collect();
        assert_eq!(starts.len(), 3);
        assert_eq!(feedback.len(), 3);
        for (record, index, stage) in [
            (feedback[0], 2, "presented"),
            (feedback[1], 1, "discarded"),
            (feedback[2], 0, "presented"),
        ] {
            assert_eq!(record["stage"], stage);
            assert!(starts[index]["draw_seq"].as_u64().is_some());
            assert_eq!(record["draw_seq"], starts[index]["draw_seq"]);
            assert_eq!(record["frame"], 7);
            assert_eq!(record["presentation"], 100);
        }
        assert_ne!(starts[0]["draw_seq"], starts[1]["draw_seq"]);
        assert_ne!(starts[1]["draw_seq"], starts[2]["draw_seq"]);
        assert_ne!(starts[0]["draw_seq"], starts[2]["draw_seq"]);
        return;
    }
    let mut submissions = Vec::new();
    for serial in 1..=3 {
        let _draw = DrawScope::begin(7, PresentationId::new(100), "repaint", "Redisplay".into());
        present_trace::record(Stage::RenderStart, 7, PresentationId::new(100));
        let mut submission = submission(serial, 100);
        submission.draw_seq = present_trace::current_draw();
        submissions.push(submission);
    }
    assert_eq!(present_trace::current_draw(), None);
    let mut receipts = Receipts {
        path: None,
        latest_presented: 0,
        clock_id: Some(1),
        pending: PendingReceipts::default(),
    };
    for submission in submissions.into_iter().rev() {
        receipts.observe(if submission.serial == 2 {
            NativeFeedback::Discarded(submission)
        } else {
            NativeFeedback::Presented(ConfirmedPresentation {
                submission,
                timestamp: CompositorTimestamp {
                    clock_id: 1,
                    seconds: 42,
                    nanoseconds: 123,
                },
            })
        });
    }
    present_trace::shutdown();
}

#[test]
fn receipt_identifies_confirmed_layout_not_latest_requested_layout() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("receipt");
    let mut receipts = Receipts {
        path: Some(path.clone()),
        latest_presented: 0,
        clock_id: Some(1),
        pending: PendingReceipts::default(),
    };
    let confirmed = |submission| {
        NativeFeedback::Presented(ConfirmedPresentation {
            submission,
            timestamp: CompositorTimestamp {
                clock_id: 1,
                seconds: 42,
                nanoseconds: 123,
            },
        })
    };
    receipts.observe(NativeFeedback::Discarded(submission(2, 200)));
    assert!(!path.exists());
    receipts.observe(confirmed(submission(1, 100)));
    let first = fs::read_to_string(&path).unwrap();
    assert!(first.contains(":submission 1 :frame 7 :presentation 100 "));
    receipts.observe(confirmed(submission(3, 300)));
    let latest = fs::read_to_string(&path).unwrap();
    assert!(latest.contains(":submission 3 :frame 7 :presentation 300 "));
    receipts.observe(confirmed(submission(1, 100)));
    receipts.observe(NativeFeedback::Discarded(submission(4, 400)));
    assert_eq!(fs::read_to_string(&path).unwrap(), latest);
}
