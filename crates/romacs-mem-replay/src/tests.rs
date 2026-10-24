use super::*;

#[test]
fn normal_pass_leak_fail_forced_pass() {
    let (normal, leak, forced) = classify_suite();
    assert_eq!(normal, Verdict::Pass);
    assert_eq!(leak, Verdict::Fail);
    assert_eq!(forced, Verdict::Pass);
}

fn fixture() -> Report {
    let mut samples = Vec::new();
    for seq in 1..=1000 {
        samples.push(Sample {
            logical_seconds: seq as f64 * 7.2,
            committed_bytes: 65536,
            record: GcRecord {
                seq,
                trigger: if seq == 1 {
                    Trigger::Explicit
                } else if seq == 2 {
                    Trigger::Idle
                } else {
                    Trigger::Threshold
                },
                core_stop_ns: 1,
                input_blocking_ns: 1,
                gc_elapsed_ns: 1,
                accounted_before: 200,
                accounted_after: 100,
                overshoot: 1,
            },
        });
    }
    Report {
        arm: Arm::Natural,
        samples,
        stats: GcStats {
            count: 1000,
            sum_ns: 1000,
            max_ns: 1,
            max_overshoot: 1,
        },
        epoch: 1000,
        reclaimed: [1; KINDS],
        payload_drops: 1,
        released_pages: 1,
        planted_bytes: 0,
        wall_ns: 1000,
    }
}
#[test]
fn rejects_vacuity_missing_records_and_online_drift() {
    let mut r = fixture();
    assert_eq!(classify(&r), Verdict::Pass);
    r.samples.remove(3);
    assert_eq!(classify(&r), Verdict::Fail);
    let mut r = fixture();
    r.reclaimed[3] = 0;
    assert_eq!(classify(&r), Verdict::Fail);
    let mut r = fixture();
    r.stats.max_ns = 2;
    assert_eq!(classify(&r), Verdict::Fail);
    let mut r = fixture();
    r.samples[1].record.trigger = Trigger::Threshold;
    assert_eq!(classify(&r), Verdict::Fail);
    let mut r = fixture();
    r.samples.clear();
    assert_eq!(classify(&r), Verdict::Fail);
}
#[test]
fn full_record_quantiles_and_early_outlier() {
    let mut r = fixture();
    r.samples[2].record.gc_elapsed_ns = 20_000_000;
    r.stats.sum_ns += 19_999_999;
    r.stats.max_ns = 20_000_000;
    assert_eq!(classify(&r), Verdict::Pass); // no real-time pause gate
    assert_eq!(p99(&r), Some(1));
    assert_eq!(r.stats.max_ns, 20_000_000);
    r.stats.max_ns = r.samples.last().unwrap().record.gc_elapsed_ns;
    assert_eq!(classify(&r), Verdict::Fail);
}
#[test]
fn logical_slope_and_windows_have_known_boundaries() {
    let mut r = fixture();
    for s in &mut r.samples {
        s.record.accounted_after = s.logical_seconds as usize * 1048576;
    }
    assert!((slope(&r).unwrap() - 3600.0).abs() < 1.0);
    assert!(!stable(windows(&r.samples, false)));
    assert!(stable(Some([100.0, 110.0, 100.0, 100.0])));
    assert!(!stable(Some([100.0, 111.0, 100.0, 100.0])));
    assert!(!stable(None));
}
