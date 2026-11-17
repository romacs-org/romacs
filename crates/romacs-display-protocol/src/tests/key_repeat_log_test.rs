use super::*;

#[test]
fn buckets_are_the_usage_logs_doubling_gap_bounds() {
    assert_eq!([0, 1, 2, 3, 4, 255, 256, 511, 512, 32767, 32768, u64::MAX].map(bucket),
               [0, 1, 2, 2, 3, 8, 9, 9, 10, 15, 16, 16]);
}

#[test]
fn a_take_returns_the_counts_since_the_last_and_starts_again() {
    let log = KeyRepeatLog::default();
    log.repeat(Duration::from_millis(260));
    log.repeat(Duration::from_millis(290));
    log.repeat(Duration::from_millis(600));
    log.released(Duration::from_millis(80));
    log.released(Duration::from_millis(640));
    log.released(Duration::from_millis(100));
    let mut after_press = [0; BUCKETS];
    after_press[9] = 2;
    after_press[10] = 1;
    assert_eq!(log.take(), KeyRepeatCounts { repeats: 3, after_press, longest_hold_ms: 640 });
    assert_eq!(log.take(), KeyRepeatCounts::default());
}
