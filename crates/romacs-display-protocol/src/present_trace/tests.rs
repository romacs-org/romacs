use super::*;

#[cfg(target_os = "linux")]
fn trace_child(name: &str, enabled: bool) -> Option<Vec<serde_json::Value>> {
    const CHILD: &str = "ROMACS_S0_TRACE_CHILD";
    if std::env::var_os(CHILD).is_some() {
        return None;
    }
    let path = std::env::temp_dir().join(format!(
        "romacs-s0-{name}-{}-{}.jsonl",
        std::process::id(),
        clock_ns(libc::CLOCK_MONOTONIC).unwrap()
    ));
    let mut child = std::process::Command::new(std::env::current_exe().unwrap());
    child
        .args(["--exact", &format!("present_trace::tests::{name}")])
        .env(CHILD, "1")
        .env_remove("ROMACS_INPUT_LATENCY_FILE")
        .env_remove("ROMACS_PRESENT_TRACE");
    if enabled {
        child.env("ROMACS_PRESENT_TRACE", &path);
    }
    let output = child.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    if !enabled {
        assert!(!path.exists());
        return Some(Vec::new());
    }
    let contents = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    Some(
        contents
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect(),
    )
}

#[cfg(target_os = "linux")]
#[test]
fn span_records_nested_drop_and_unwind_with_exact_identities() {
    if let Some(records) = trace_child(
        "span_records_nested_drop_and_unwind_with_exact_identities",
        true,
    ) {
        let stages: Vec<_> = records
            .iter()
            .map(|r| r["stage"].as_str().unwrap())
            .collect();
        assert_eq!(
            stages,
            [
                "redisplay_start",
                "fontify_start",
                "fontify_end",
                "redisplay_end"
            ]
        );
        for (index, record) in records.iter().enumerate() {
            assert_eq!(
                record["frame"],
                if index == 0 || index == 3 { 7 } else { 8 }
            );
            assert_eq!(
                record["presentation"],
                if index == 0 || index == 3 { 42 } else { 0 }
            );
            assert_eq!(record["clock_id"], libc::CLOCK_MONOTONIC as u32);
        }
        assert!(
            records
                .windows(2)
                .all(|r| r[0]["ns"].as_u64() <= r[1]["ns"].as_u64())
        );
        return;
    }
    init();
    let _ = std::panic::catch_unwind(|| {
        let _outer = Span::new(
            Stage::RedisplayStart,
            Stage::RedisplayEnd,
            7,
            Some(PresentationId::new(42)),
        );
        let _inner = Span::new(Stage::FontifyStart, Stage::FontifyEnd, 8, None);
        panic!("exercise scope unwind");
    });
    shutdown();
}

#[cfg(target_os = "linux")]
#[test]
fn span_disabled_does_not_create_trace() {
    if trace_child("span_disabled_does_not_create_trace", false).is_some() {
        return;
    }
    let _span = Span::new(Stage::RedisplayStart, Stage::RedisplayEnd, 7, None);
    assert!(!enabled());
    shutdown();
}

#[cfg(target_os = "linux")]
#[test]
fn present_trace_only_input_receive_is_enabled_and_preserves_domain() {
    if let Some(records) = trace_child(
        "present_trace_only_input_receive_is_enabled_and_preserves_domain",
        true,
    ) {
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["stage"], "input_received");
        assert_eq!(records[0]["frame"], 7);
        assert_eq!(records[0]["presentation"], 0);
        assert_eq!(records[0]["input_sequence"], 1);
        assert_eq!(records[0]["clock_id"], 17);
        assert_eq!(records[0]["ns"], 123);
        return;
    }
    assert!(
        crate::input_latency::enabled(),
        "presentation tracing must enable input capture independently"
    );
    assert!(
        crate::input_latency::received(
            7,
            "key",
            crate::input_latency::PlatformTimestamp {
                clock_id: 17,
                nanoseconds: 123
            }
        )
        .is_some()
    );
    shutdown();
}

#[cfg(target_os = "linux")]
#[test]
fn command_span_keeps_consumed_sequence_metadata_after_completion() {
    if let Some(records) = trace_child(
        "command_span_keeps_consumed_sequence_metadata_after_completion",
        true,
    ) {
        let stages: Vec<_> = records
            .iter()
            .map(|r| r["stage"].as_str().unwrap())
            .collect();
        assert_eq!(
            stages,
            [
                "input_received",
                "input_received",
                "command_start",
                "command_end",
                "command_start",
                "command_end"
            ]
        );
        assert_eq!(records[0]["input_sequence"], 1);
        assert_eq!(records[1]["input_sequence"], 2);
        for record in &records[2..4] {
            assert_eq!(record["input_sequence"], 1);
            assert_eq!(record["input_sequence_end"], 2);
            assert_eq!(record["frame"], 7);
            assert_eq!(record["presentation"], 0);
        }
        for record in &records[4..] {
            assert!(record.get("input_sequence").is_none());
            assert!(record.get("input_sequence_end").is_none());
        }
        return;
    }
    use crate::input_latency::{self, CommandInputs, PlatformTimestamp};
    for ns in [123, 124] {
        let token = input_latency::received(
            7,
            "key",
            PlatformTimestamp {
                clock_id: 1,
                nanoseconds: ns,
            },
        )
        .unwrap();
        input_latency::consumed(token);
    }
    let command = CommandInputs::begin();
    let span = command.trace_span(7);
    command.complete(|_| Vec::new());
    drop(span);
    let empty = CommandInputs::begin();
    let span = empty.trace_span(7);
    drop(empty);
    drop(span);
    shutdown();
}

#[cfg(target_os = "linux")]
#[test]
fn draw_cookie_survives_feedback_reordering_and_scope_exit() {
    let name = "draw_cookie_survives_feedback_reordering_and_scope_exit";
    if let Some(records) = trace_child(name, true) {
        let starts: Vec<_> = records
            .iter()
            .filter(|r| r["stage"] == "render_start")
            .collect();
        let feedback: Vec<_> = records
            .iter()
            .filter(|r| r["stage"] == "presented")
            .collect();
        assert_eq!(starts.len(), 2);
        assert_eq!(feedback.len(), 2);
        assert_ne!(starts[0]["draw_seq"], starts[1]["draw_seq"]);
        assert_eq!(feedback[0]["draw_seq"], starts[1]["draw_seq"]);
        assert_eq!(feedback[1]["draw_seq"], starts[0]["draw_seq"]);
        assert_eq!(starts[0]["plan"], "repaint");
        assert_eq!(starts[0]["reasons"], "Redisplay");
        for record in records.iter().filter(|r| r["stage"] != "presented") {
            assert!(record["draw_seq"].as_u64().is_some());
        }
        return;
    }
    let mut cookies = Vec::new();
    for _ in 0..2 {
        let _draw = DrawScope::begin(7, PresentationId::new(42), "repaint", "Redisplay".into());
        cookies.push(current_draw());
        for stage in [
            Stage::AcquireStart,
            Stage::AcquireEnd,
            Stage::RenderStart,
            Stage::SubmitStart,
            Stage::SubmitEnd,
            Stage::Submit,
            Stage::PresentCallStart,
            Stage::PresentCallEnd,
            Stage::Present,
        ] {
            record(stage, 7, PresentationId::new(42));
        }
    }
    assert_eq!(current_draw(), None);
    std::thread::spawn(move || {
        for cookie in cookies.into_iter().rev() {
            presented_draw(7, PresentationId::new(42), 17, 999, cookie);
        }
    })
    .join()
    .unwrap();
    shutdown();
}

fn event(stage: Stage) -> Record {
    Record {
        stage,
        frame: 7,
        presentation: 42,
        clock_id: 1,
        ns: 123,
        realtime_ns: (stage == Stage::Publish).then_some(456),
        input_sequence: None,
        input_sequence_end: None,
        draw_seq: None,
        detail: None,
        plan: None,
        reasons: None,
    }
}

fn queue(capacity: usize) -> (Recorder, Receiver<Record>) {
    let (events, incoming) = bounded(capacity);
    let (stop, _) = bounded(1);
    (
        Recorder {
            events,
            dropped: Arc::new(AtomicU64::new(0)),
            stop,
            writer: Mutex::new(None),
        },
        incoming,
    )
}

#[test]
fn env_parsing_accepts_only_absolute_nonempty_paths() {
    assert_eq!(parse_path(None), Ok(None));
    assert_eq!(parse_path(Some(OsString::new())), Ok(None));
    assert!(parse_path(Some("trace.jsonl".into())).is_err());
    assert!(parse_path(Some(" ./trace.jsonl".into())).is_err());
    assert_eq!(
        parse_path(Some("/run/trace.jsonl".into())),
        Ok(Some(PathBuf::from("/run/trace.jsonl")))
    );
}

#[test]
fn disabled_path_does_not_sample_or_send() {
    record_to(None, Stage::Publish, 7, PresentationId::new(42), |_| {
        panic!("disabled tracing must not sample a clock")
    });
}

#[test]
fn formatting_preserves_identity_domain_and_paired_publish_clocks() {
    let mut bytes = Vec::new();
    write_record(&mut bytes, &event(Stage::Publish)).unwrap();
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        "{\"stage\":\"publish\",\"frame\":7,\"presentation\":42,\"clock_id\":1,\"ns\":123,\"realtime_ns\":456}\n"
    );
}

#[test]
fn stage_labels_and_optional_realtime_are_unambiguous() {
    for (stage, label) in [
        (Stage::RedisplayStart, "redisplay_start"),
        (Stage::RedisplayEnd, "redisplay_end"),
        (Stage::ResolveLayoutStart, "resolve_layout_start"),
        (Stage::ResolveLayoutEnd, "resolve_layout_end"),
        (Stage::FontifyStart, "fontify_start"),
        (Stage::FontifyEnd, "fontify_end"),
        (Stage::ModeLineStart, "mode_line_start"),
        (Stage::ModeLineEnd, "mode_line_end"),
        (Stage::ResolveStart, "resolve_start"),
        (Stage::ResolveEnd, "resolve_end"),
        (Stage::CoverageRetry, "coverage_retry"),
        (Stage::CommandStart, "command_start"),
        (Stage::CommandEnd, "command_end"),
        (Stage::InputReceived, "input_received"),
        (Stage::FlattenStart, "flatten_start"),
        (Stage::FlattenEnd, "flatten_end"),
        (Stage::MaterializeStart, "materialize_start"),
        (Stage::MaterializeEnd, "materialize_end"),
        (Stage::AcquireStart, "acquire_start"),
        (Stage::AcquireEnd, "acquire_end"),
        (Stage::TessellateStart, "tessellate_start"),
        (Stage::TessellateEnd, "tessellate_end"),
        (Stage::AtlasUploadStart, "atlas_upload_start"),
        (Stage::AtlasUploadEnd, "atlas_upload_end"),
        (Stage::NonRootStart, "non_root_start"),
        (Stage::NonRootEnd, "non_root_end"),
        (Stage::GpuDone, "gpu_done"),
        (Stage::Prepared, "prepared"),
        (Stage::RenderStart, "render_start"),
        (Stage::Submit, "submit"),
        (Stage::Present, "present"),
        (Stage::Presented, "presented"),
        (Stage::Superseded, "superseded"),
        (Stage::Discarded, "discarded"),
    ] {
        let mut bytes = Vec::new();
        write_record(&mut bytes, &event(stage)).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["stage"], label);
        assert!(value.get("realtime_ns").is_none());
    }
}

#[test]
fn full_channel_drops_without_replacing_queued_evidence() {
    let (recorder, incoming) = queue(1);
    recorder.send(event(Stage::Publish));
    recorder.send(event(Stage::Prepared));
    recorder.send(event(Stage::Present));
    assert_eq!(recorder.dropped.load(Ordering::Relaxed), 2);
    assert_eq!(incoming.try_recv().unwrap().stage, Stage::Publish);
    assert!(incoming.try_recv().is_err());
    let mut bytes = Vec::new();
    flush_records(&mut bytes, &recorder.dropped).unwrap();
    assert_eq!(bytes, b"{\"stage\":\"dropped\",\"count\":2}\n");
    assert_eq!(recorder.dropped.load(Ordering::Relaxed), 0);
    flush_records(&mut bytes, &recorder.dropped).unwrap();
    assert_eq!(bytes, b"{\"stage\":\"dropped\",\"count\":2}\n");
}

#[test]
fn disconnected_writer_counts_unsent_records() {
    let (recorder, incoming) = queue(1);
    drop(incoming);
    recorder.send(event(Stage::Present));
    assert_eq!(recorder.dropped.load(Ordering::Relaxed), 1);
}

#[test]
fn failed_clock_sample_is_counted_not_recorded_as_zero() {
    let (recorder, incoming) = queue(1);
    record_to(
        Some(&recorder),
        Stage::Publish,
        7,
        PresentationId::new(42),
        |_| None,
    );
    assert_eq!(recorder.dropped.load(Ordering::Relaxed), 1);
    assert!(incoming.try_recv().is_err());
}

#[test]
fn record_samples_only_after_enablement() {
    let (recorder, incoming) = queue(1);
    record_to(
        Some(&recorder),
        Stage::Publish,
        7,
        PresentationId::new(42),
        |stage| {
            assert_eq!(stage, Stage::Publish);
            Some((1, 123, Some(456)))
        },
    );
    let record = incoming.try_recv().unwrap();
    assert_eq!(record.presentation, 42);
    assert_eq!(record.realtime_ns, Some(456));
}

#[test]
fn writer_drains_and_flushes_at_normal_shutdown() {
    let (events, incoming) = bounded(2);
    let (stop, stopped) = bounded(1);
    events.try_send(event(Stage::Publish)).unwrap();
    events.try_send(event(Stage::Prepared)).unwrap();
    stop.try_send(()).unwrap();
    let dropped = AtomicU64::new(3);
    let mut bytes = Vec::new();
    write_records(BufWriter::new(&mut bytes), incoming, stopped, &dropped).unwrap();
    let lines: Vec<_> = bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect();
    assert_eq!(lines.len(), 3);
    let stages: Vec<_> = lines
        .iter()
        .map(|line| serde_json::from_slice::<serde_json::Value>(line).unwrap()["stage"].clone())
        .collect();
    assert_eq!(stages, ["publish", "prepared", "dropped"]);
}

#[test]
fn deferred_stamp_keeps_its_sample_time() {
    let (recorder, incoming) = queue(2);
    let record = stamp_with(
        &recorder,
        Stage::Publish,
        7,
        PresentationId::new(42),
        |_| Some((1, 123, Some(456))),
    )
    .unwrap();
    assert!(incoming.try_recv().is_err(), "stamping must not queue");
    recorder.send(record);
    let queued = incoming.try_recv().unwrap();
    assert_eq!((queued.ns, queued.realtime_ns), (123, Some(456)));
}

#[test]
fn writer_stops_promptly_with_an_empty_queue() {
    let (_events, incoming) = bounded::<Record>(1);
    let (stop, stopped) = bounded(1);
    stop.try_send(()).unwrap();
    let mut bytes = Vec::new();
    write_records(&mut bytes, incoming, stopped, &AtomicU64::new(0)).unwrap();
    assert!(bytes.is_empty());
}

#[test]
fn writer_flushes_when_producers_disconnect() {
    let (events, incoming) = bounded(1);
    let (_stop, stopped) = bounded(1);
    events.try_send(event(Stage::Presented)).unwrap();
    drop(events);
    let mut bytes = Vec::new();
    write_records(
        BufWriter::new(&mut bytes),
        incoming,
        stopped,
        &AtomicU64::new(0),
    )
    .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["stage"],
        "presented"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn environment_trace_writes_and_drains_jsonl() {
    const CHILD: &str = "ROMACS_PRESENT_TRACE_TEST_CHILD";
    if let Ok(expected) = std::env::var(CHILD) {
        assert_eq!(enabled(), expected == "enabled");
        for stage in [
            Stage::Publish,
            Stage::Prepared,
            Stage::RenderStart,
            Stage::Submit,
            Stage::Present,
            Stage::Superseded,
            Stage::Discarded,
        ] {
            if stage == Stage::Superseded {
                // The deferred form queues exactly what it sampled.
                if let Some(stamp) = stamp(stage, 7, PresentationId::new(42)) {
                    stamp.emit();
                }
            } else {
                record(stage, 7, PresentationId::new(42));
            }
        }
        presented(7, PresentationId::new(42), 17, 999);
        shutdown();
        return;
    }

    // Separate processes isolate the cached environment without mutating the
    // environment of concurrent tests. No GUI or compositor is involved.
    let path = std::env::temp_dir().join(format!(
        "romacs-present-trace-{}-{}.jsonl",
        std::process::id(),
        clock_ns(libc::CLOCK_MONOTONIC).unwrap()
    ));
    for configured in [
        None,
        Some(OsString::new()),
        Some("relative.jsonl".into()),
        Some(path.clone().into_os_string()),
    ] {
        let expected = if configured.as_ref() == Some(&path.clone().into_os_string()) {
            "enabled"
        } else {
            "disabled"
        };
        let mut child = std::process::Command::new(std::env::current_exe().unwrap());
        child
            .arg("--exact")
            .arg("present_trace::tests::environment_trace_writes_and_drains_jsonl")
            .env(CHILD, expected)
            .env_remove("ROMACS_PRESENT_TRACE");
        if let Some(value) = configured {
            child.env("ROMACS_PRESENT_TRACE", value);
        }
        let output = child.output().unwrap();
        assert!(output.status.success(), "{output:?}");
        if expected == "disabled" {
            assert!(!path.exists());
        }
    }
    let contents = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    let records: Vec<serde_json::Value> = contents
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 8);
    for record in &records {
        assert_eq!(record["frame"], 7);
        assert_eq!(record["presentation"], 42);
    }
    assert_eq!(records[0]["stage"], "publish");
    assert!(records[0]["realtime_ns"].is_u64());
    assert_eq!(records[7]["stage"], "presented");
    assert_eq!(records[7]["clock_id"], 17);
    assert_eq!(records[7]["ns"], 999);
}

#[cfg(target_os = "linux")]
#[test]
fn local_stages_share_monotonic_clock_and_publish_pairs_realtime() {
    let (clock, ns, realtime) = sample(Stage::Publish).unwrap();
    let (next_clock, next_ns, next_realtime) = sample(Stage::Prepared).unwrap();
    assert_eq!(clock, libc::CLOCK_MONOTONIC as u32);
    assert_eq!(next_clock, clock);
    assert!(next_ns >= ns);
    assert!(realtime.unwrap() > ns);
    assert_eq!(next_realtime, None);
}
